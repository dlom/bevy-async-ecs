mod command;
mod system;

use crate::command::BoxedCommand;
use crate::system::{AsyncSystem, SystemOperation};
use async_channel::{Receiver, Sender, TryRecvError};
use bevy::app::{App, First, Plugin};
use bevy::core::Name;
use bevy::ecs::component::Component;
use bevy::ecs::entity::Entity;
use bevy::ecs::schedule::{apply_deferred, IntoSystemConfigs};
use bevy::ecs::system::{Command, Commands, Query, ResMut, Resource};
use bevy::ecs::world::{FromWorld, World};
use bevy::log::debug;
use bevy::prelude::IntoSystem;

pub struct AsyncEcsPlugin;

impl Plugin for AsyncEcsPlugin {
	fn build(&self, app: &mut App) {
		app.init_resource::<OperationQueue>().add_systems(
			First,
			(receive_operations, apply_operations, apply_deferred).chain(),
		);
	}
}

enum AsyncOperation {
	Command(BoxedCommand),
	System(SystemOperation),
	Entity,
	Event,
	Resource,
}

impl Command for AsyncOperation {
	fn apply(self, world: &mut World) {
		match self {
			AsyncOperation::Command(command) => command.apply(world),
			AsyncOperation::System(system_op) => system_op.apply(world),
			// AsyncOperation::Entity => {}
			// AsyncOperation::Event => {}
			// AsyncOperation::Resource => {}
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
struct OperationSender(Sender<AsyncOperation>);

impl From<Sender<AsyncOperation>> for OperationSender {
	fn from(sender: Sender<AsyncOperation>) -> Self {
		Self(sender)
	}
}

impl OperationSender {
	async fn send(&self, operation: AsyncOperation) {
		self.0.send(operation).await.expect("invariant broken");
	}
}

#[derive(Clone)]
pub struct AsyncWorld(OperationSender);

impl AsyncWorld {
	pub async fn apply_command<C: Command>(&self, command: C) {
		let operation = AsyncOperation::Command(BoxedCommand::new(command));
		self.0.send(operation).await;
	}

	pub async fn register_system<M>(&self, system: impl IntoSystem<(), (), M>) -> AsyncSystem {
		let system = Box::new(IntoSystem::into_system(system));
		AsyncSystem::new(system, self.0.clone()).await
	}
}

impl FromWorld for AsyncWorld {
	fn from_world(world: &mut World) -> Self {
		let (sender, receiver) = async_channel::unbounded();
		world.spawn((OperationReceiver(receiver), Name::new("AsyncReceiver")));
		Self(sender.into())
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use bevy::MinimalPlugins;

	#[derive(Component)]
	struct Marker;

	impl OperationSender {
		fn send_blocking(&self, operation: AsyncOperation) {
			self.0.send_blocking(operation).unwrap();
		}
	}

	#[test]
	fn command() {
		let mut app = App::new();
		app.add_plugins((MinimalPlugins, AsyncEcsPlugin));

		let world = AsyncWorld::from_world(&mut app.world);
		let (sender, receiver) = async_channel::bounded(1);
		let command = BoxedCommand::new(move |world: &mut World| {
			let id = world.spawn(Marker).id();
			sender.send_blocking(id).unwrap();
		});

		world.0.send_blocking(AsyncOperation::Command(command));
		app.update();

		let id = receiver.recv_blocking().unwrap();
		assert!(app.world.entity(id).get::<Marker>().is_some());
	}
}
