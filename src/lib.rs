mod command;

use crate::command::BoxedCommand;
use async_channel::{Receiver, Sender, TryRecvError};
use bevy_app::{App, Last, Plugin};
use bevy_core::Name;
use bevy_ecs::component::Component;
use bevy_ecs::entity::Entity;
use bevy_ecs::schedule::{apply_deferred, IntoSystemConfigs};
use bevy_ecs::system::{Command, Commands, Query, ResMut, Resource};
use bevy_ecs::world::{FromWorld, World};
use bevy_log::debug;

pub struct AsyncEcsPlugin;

impl Plugin for AsyncEcsPlugin {
	fn build(&self, app: &mut App) {
		app.init_resource::<OperationQueue>().add_systems(
			Last,
			(receive_operations, apply_operations, apply_deferred).chain(),
		);
	}
}

enum AsyncOperation {
	Command(BoxedCommand),
	SystemOperation,
	EntityOperation,
	EventOperation,
	ResourceOperation,
}

impl Command for AsyncOperation {
	fn apply(self, world: &mut World) {
		match self {
			AsyncOperation::Command(command) => command.apply(world),
			AsyncOperation::SystemOperation => {
				//
				todo!()
			}
			// AsyncOperation::EntityOperation => {}
			// AsyncOperation::EventOperation => {}
			// AsyncOperation::ResourceOperation => {}
			_ => unimplemented!(),
		}
	}
}

#[derive(Resource)]
struct OperationQueue(Vec<AsyncOperation>);

const DEFAULT_QUEUE_SIZE: usize = 16;

impl Default for OperationQueue {
	fn default() -> Self {
		Self(Vec::with_capacity(DEFAULT_QUEUE_SIZE))
	}
}

fn receive_operations(
	mut commands: Commands,
	receivers: Query<(Entity, &OperationReceiver)>,
	mut queue: ResMut<OperationQueue>,
) {
	debug_assert_eq!(0, queue.0.len());
	debug_assert!(queue.0.capacity() >= DEFAULT_QUEUE_SIZE);

	queue.0.clear();

	for (id, receiver) in receivers.iter() {
		if receiver.enqueue_into(&mut queue).is_err() {
			commands.entity(id).despawn()
		}
	}
}

fn apply_operations(world: &mut World) {
	world.resource_scope::<OperationQueue, _>(|world, mut queue| {
		for operation in queue.0.drain(..) {
			operation.apply(world);
		}
	})
}

#[derive(Component)]
struct OperationReceiver(Receiver<AsyncOperation>);

impl OperationReceiver {
	fn enqueue_into(&self, queue: &mut OperationQueue) -> Result<(), ()> {
		loop {
			match self.0.try_recv() {
				Ok(system) => queue.0.push(system),
				Err(TryRecvError::Closed) => {
					debug!("command receiver closed");
					break Err(());
				}
				Err(TryRecvError::Empty) => break Ok(()),
			}
		}
	}
}

#[derive(Clone)]
pub struct AsyncWorld(Sender<AsyncOperation>);

impl FromWorld for AsyncWorld {
	fn from_world(world: &mut World) -> Self {
		let (sender, receiver) = async_channel::unbounded();
		world.spawn((OperationReceiver(receiver), Name::new("AsyncReceiver")));
		Self(sender)
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use bevy_app::ScheduleRunnerPlugin;
	use bevy_core::TypeRegistrationPlugin;
	// use futures_lite::future;

	#[derive(Component)]
	struct Counter(u8);

	macro_rules! assert_counter {
		($id:expr, $value:expr, $world:expr) => {
			assert_eq!($value, $world.entity($id).get::<Counter>().unwrap().0);
		};
	}

	fn increase_counter(mut query: Query<&mut Counter>) {
		for mut counter in query.iter_mut() {
			counter.0 += 1;
		}
	}

	#[test]
	fn command() {
		let mut app = App::new();
		app.add_plugins((ScheduleRunnerPlugin::default(), AsyncEcsPlugin));

		let world = AsyncWorld::from_world(&mut app.world);
		let (sender, receiver) = async_channel::bounded(1);
		let command = BoxedCommand::new(move |world: &mut World| {
			let id = world.spawn((Counter(0))).id();
			sender.send_blocking(id).unwrap();
		});

		world
			.0
			.send_blocking(AsyncOperation::Command(command))
			.unwrap();
		app.update();

		let id = receiver.recv_blocking().unwrap();
		assert_counter!(id, 0, &app.world);
	}
}
