use async_channel::{Receiver, Sender, TryRecvError};
use bevy_ecs::prelude::*;
use bevy_ecs::system::Command;

pub use super::command::BoxedCommand;
pub use super::entity::reflect::ReflectOperation;
pub use super::entity::EntityOperation;
pub use super::resource::ResourceOperation;
pub use super::system::SystemOperation;

/// An operation that can be applied to an `AsyncWorld`.
#[derive(Debug)]
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
#[derive(Debug)]
pub struct OperationQueue(Vec<AsyncOperation>);

impl Default for OperationQueue {
	fn default() -> Self {
		Self::new()
	}
}

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

	#[cfg(test)]
	pub(crate) fn len(&self) -> usize {
		self.0.len()
	}
}

impl Command for OperationQueue {
	fn apply(self, world: &mut World) {
		for operation in self.0 {
			operation.apply(world);
		}
	}
}

impl FromIterator<AsyncOperation> for OperationQueue {
	fn from_iter<I: IntoIterator<Item = AsyncOperation>>(iter: I) -> Self {
		Self(iter.into_iter().collect())
	}
}

impl From<OperationQueue> for AsyncOperation {
	fn from(queue: OperationQueue) -> Self {
		Self::Queue(queue)
	}
}

/// Use this to send `Operation`s directly to the Bevy `World`, where they will be applied during
/// the `Last` schedule.
#[derive(Clone, Debug)]
pub struct OperationSender(Sender<AsyncOperation>);

impl OperationSender {
	/// Sends an `Operation` directly to the Bevy `World`, where they will be applied during
	/// the `Last` schedule.
	pub async fn send<O: Into<AsyncOperation>>(&self, operation: O) {
		let operation = operation.into();
		self.send_inner(operation).await;
	}

	async fn send_inner(&self, operation: AsyncOperation) {
		self.0.send(operation).await.expect("invariant broken");
	}
}

impl From<Sender<AsyncOperation>> for OperationSender {
	fn from(sender: Sender<AsyncOperation>) -> Self {
		Self(sender)
	}
}

#[derive(Component)]
pub(crate) struct OperationReceiver(Receiver<AsyncOperation>);

impl OperationReceiver {
	fn enqueue_into(&self, queue: &mut WorldOperationQueue) -> Result<(), ()> {
		loop {
			match self.0.try_recv() {
				Ok(system) => queue.0.push(system),
				Err(TryRecvError::Closed) => break Err(()),
				Err(TryRecvError::Empty) => break Ok(()),
			}
		}
	}
}

impl From<Receiver<AsyncOperation>> for OperationReceiver {
	fn from(receiver: Receiver<AsyncOperation>) -> Self {
		Self(receiver)
	}
}

#[derive(Resource)]
pub(crate) struct WorldOperationQueue(Vec<AsyncOperation>);

impl Default for WorldOperationQueue {
	fn default() -> Self {
		Self(Vec::with_capacity(16))
	}
}

pub(crate) fn receive_operations(
	mut commands: Commands,
	receivers: Query<(Entity, &OperationReceiver)>,
	mut queue: ResMut<WorldOperationQueue>,
) {
	for (id, receiver) in receivers.iter() {
		if receiver.enqueue_into(&mut queue).is_err() {
			commands.entity(id).despawn()
		}
	}
}

pub(crate) fn apply_operations(world: &mut World) {
	world.resource_scope::<WorldOperationQueue, _>(|world, mut queue| {
		for operation in queue.0.drain(..) {
			operation.apply(world);
		}
	})
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::{AsyncEcsPlugin, AsyncEntity, AsyncWorld};
	use bevy::prelude::*;
	use futures_lite::future;
	use std::any::TypeId;

	#[derive(Default, Component, Reflect)]
	#[reflect(Component)]
	struct Counter(u8);

	#[test]
	fn queue() {
		let mut app = App::new();
		app.register_type::<Counter>();
		app.add_plugins((MinimalPlugins, AsyncEcsPlugin));

		let (value_tx, value_rx) = async_channel::bounded(1);
		let async_world = AsyncWorld::from_world(&mut app.world);
		let id = app.world.spawn_empty().id();

		let type_id = TypeId::of::<Counter>();

		std::thread::spawn(move || {
			future::block_on(async move {
				let counter = Box::new(Counter(3));

				let operation = OperationQueue::from_iter([
					ReflectOperation::InsertComponent(id, counter).into(),
					ReflectOperation::WaitForComponent(id, type_id, value_tx).into(),
					ReflectOperation::RemoveComponent(id, type_id).into(),
				]);

				async_world.apply_operation(operation.into()).await;
			});
		});

		let value = loop {
			match value_rx.try_recv() {
				Ok(value) => break value,
				Err(_) => app.update(),
			}
		};
		app.update();

		let counter = Counter::take_from_reflect(value).unwrap();
		assert_eq!(3, counter.0);
		assert!(app.world.entity(id).get::<Counter>().is_none());
	}

	#[test]
	fn coverage() {
		let id = Entity::PLACEHOLDER;

		let queue1 = {
			let mut queue = OperationQueue::default();
			queue.push(EntityOperation::Despawn(id));
			queue
		};

		let queue2 = OperationQueue::from_iter([EntityOperation::Despawn(id).into()]);

		assert_eq!(queue1.len(), queue2.len());
	}

	#[test]
	fn sender() {
		let mut app = App::new();
		app.add_plugins((MinimalPlugins, AsyncEcsPlugin));

		let async_world = AsyncWorld::from_world(&mut app.world);
		let sender = async_world.sender();
		let entity = AsyncEntity::new(Entity::PLACEHOLDER, sender.clone());
		let other_sender = entity.sender();
		assert_eq!(4, sender.0.sender_count());
		assert_eq!(4, other_sender.0.sender_count());
	}
}
