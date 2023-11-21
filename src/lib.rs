mod command;
mod entity;
mod system;
mod world;

use crate::command::BoxedCommand;
use crate::entity::{wait_for_reflect_components, EntityOperation};
use crate::system::SystemOperation;
use async_channel::{Receiver, Sender, TryRecvError};
use bevy::ecs::system::Command;
use bevy::prelude::*;
use std::borrow::Cow;

pub use entity::{AsyncComponent, AsyncEntity};
pub use system::{AsyncIOSystem, AsyncSystem};
pub use world::AsyncWorld;

type CowStr = Cow<'static, str>;

pub struct AsyncEcsPlugin;

impl Plugin for AsyncEcsPlugin {
	fn build(&self, app: &mut App) {
		app.init_resource::<OperationQueue>()
			.add_systems(
				Last,
				(receive_operations, apply_operations, apply_deferred).chain(),
			)
			.add_systems(PostUpdate, wait_for_reflect_components);
	}
}

enum AsyncOperation {
	Command(BoxedCommand),
	System(SystemOperation),
	Entity(EntityOperation),
	Event,
	Resource,
}

impl Command for AsyncOperation {
	fn apply(self, world: &mut World) {
		match self {
			AsyncOperation::Command(command) => command.apply(world),
			AsyncOperation::System(system_op) => system_op.apply(world),
			AsyncOperation::Entity(entity_op) => entity_op.apply(world),
			// AsyncOperation::Event => {}
			// AsyncOperation::Resource => {}
			_ => unimplemented!(),
		}
	}
}

#[derive(Clone)]
struct OperationSender(Sender<AsyncOperation>);

impl OperationSender {
	async fn send<O: Into<AsyncOperation>>(&self, operation: O) {
		let operation = operation.into();
		self.send_inner(operation).await;
	}

	async fn send_inner(&self, operation: AsyncOperation) {
		self.0.send(operation).await.expect("invariant broken");
	}
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
