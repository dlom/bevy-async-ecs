#![warn(missing_docs)]
#![doc = include_str!("../README.md")]

mod command;
mod entity;
mod resource;
mod system;
mod world;

use crate::entity::wait_for_reflect_components;
use crate::resource::wait_for_reflect_resources;
use async_channel::{Receiver, Sender, TryRecvError};
use bevy::ecs::system::Command;
use bevy::prelude::*;
use std::borrow::Cow;

use crate::operations::AsyncOperation;
pub use entity::{AsyncComponent, AsyncEntity};
pub use resource::AsyncResource;
pub use system::{AsyncIOSystem, AsyncSystem};
pub use world::AsyncWorld;

/// Types for interacting with the `AsyncWorld` directly, rather than through the convenience commands.
pub mod operations {
	use bevy::ecs::system::Command;
	use bevy::prelude::*;

	pub use super::command::BoxedCommand;
	pub use super::entity::reflect::ReflectOperation;
	pub use super::entity::EntityOperation;
	pub use super::resource::ResourceOperation;
	pub use super::system::{AsyncIOBeacon, SystemOperation};

	/// An operation that can be applied to an `AsyncWorld`.
	#[non_exhaustive]
	pub enum AsyncOperation {
		/// A vanilla Bevy `Command` (wrapped in a `CommandBox`).
		Command(BoxedCommand),
		/// `System` operations.
		System(SystemOperation),
		/// `Entity` operations.
		Entity(EntityOperation),
		/// `Resource` operations.
		Resource(ResourceOperation),
		/// A FIFO queue of `AsyncOperation`s.
		Queue(OperationQueue),
	}

	impl Command for AsyncOperation {
		fn apply(self, world: &mut World) {
			match self {
				AsyncOperation::Command(command) => command.apply(world),
				AsyncOperation::System(system_op) => system_op.apply(world),
				AsyncOperation::Entity(entity_op) => entity_op.apply(world),
				AsyncOperation::Resource(resource_op) => resource_op.apply(world),
				AsyncOperation::Queue(queue) => queue.apply(world),
			}
		}
	}

	/// A queue of `AsyncOperation`s that will be applied to the `AsyncWorld` atomically in FIFO order.
	#[derive(Default)]
	pub struct OperationQueue(Vec<AsyncOperation>);

	impl OperationQueue {
		/// Constructs a new, empty `OperationQueue`.
		pub fn new() -> Self {
			Self(Vec::with_capacity(4))
		}

		/// Appends an operation to the queue.
		pub fn push(&mut self, operation: impl Into<AsyncOperation>) {
			let operation = operation.into();
			self.0.push(operation);
		}
	}

	impl Command for OperationQueue {
		fn apply(self, world: &mut World) {
			for operation in self.0 {
				operation.apply(world);
			}
		}
	}

	impl From<OperationQueue> for AsyncOperation {
		fn from(queue: OperationQueue) -> Self {
			Self::Queue(queue)
		}
	}
}

type CowStr = Cow<'static, str>;

/// Adds asynchronous ECS operations to Bevy `App`s.
pub struct AsyncEcsPlugin;

impl Plugin for AsyncEcsPlugin {
	fn build(&self, app: &mut App) {
		app.init_resource::<OperationQueue>()
			.add_systems(
				Last,
				(receive_operations, apply_operations, apply_deferred).chain(),
			)
			.add_systems(
				PostUpdate,
				(wait_for_reflect_components, wait_for_reflect_resources),
			);
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
