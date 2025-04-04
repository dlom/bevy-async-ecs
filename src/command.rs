use crate::die;
use async_channel::{Receiver, Sender, TryRecvError};
use bevy_ecs::prelude::*;
use bevy_ecs::world::CommandQueue;
use bevy_log::tracing::debug;
use std::fmt;

/// The object-safe equivalent of a `Box<dyn Command>`.
pub struct BoxedCommand(CommandQueue);

impl BoxedCommand {
	/// Constructs a new `BoxedCommand` from the given Bevy command.
	pub fn new<C: Command>(inner: C) -> Self {
		Self({
			let mut queue = CommandQueue::default();
			queue.push(inner);
			queue
		})
	}
}

impl fmt::Debug for BoxedCommand {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.debug_struct("BoxedCommand").finish()
	}
}

impl From<BoxedCommand> for CommandQueue {
	fn from(boxed: BoxedCommand) -> Self {
		boxed.0
	}
}

impl Command for BoxedCommand {
	fn apply(mut self, world: &mut World) {
		self.0.apply(world);
	}
}

/// Builds a `CommandQueue` that can be applied to the world that the builder was
/// constructed from.
///
/// The easiest way to get a `CommandQueueBuilder` is with `AsyncWorld::start_queue()`
pub struct CommandQueueBuilder {
	inner: CommandQueue,
	sender: CommandQueueSender,
}

impl CommandQueueBuilder {
	pub(crate) fn new(sender: CommandQueueSender) -> Self {
		let inner = CommandQueue::default();
		Self { inner, sender }
	}

	/// Push a command into the `CommandQueue`.
	///
	/// This function is meant to be chained.
	pub fn push<C: Command>(mut self, command: C) -> Self {
		self.inner.push(command);
		self
	}

	/// Apply the `CommandQueue` to the world it was constructed from.
	///
	/// This function is meant to be the end of the chain.
	pub async fn apply(self) {
		self.sender.send_queue(self.inner).await;
	}

	/// Return the built `CommandQueue` _without_ applying it to the world it was
	/// constructed from.
	///
	/// This function is meant to be the end of the chain.
	pub fn build(self) -> CommandQueue {
		self.inner
	}
}

impl fmt::Debug for CommandQueueBuilder {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.debug_struct("CommandQueueBuilder")
			.field("inner", &"[..]")
			.field("sender", &self.sender)
			.finish()
	}
}

/// Use this to send commands (stored in `CommandQueue`s) directly to the Bevy World, where they will
/// be applied during the Last schedule.
///
/// This sender internally operates on `CommandQueue`s rather than individual commands.
/// Single commands can still be sent with `CommandQueueSender::send_single()`.
///
/// The easiest way to get a `CommandQueueSender` is with `AsyncWorld::sender()`.
#[derive(Clone, Debug)]
pub struct CommandQueueSender(Sender<CommandQueue>);

impl CommandQueueSender {
	pub(crate) fn new(inner: Sender<CommandQueue>) -> Self {
		Self(inner)
	}

	/// Sends an `CommandQueue` directly to the Bevy `World`, where they will be applied during
	/// the `Last` schedule.
	pub async fn send_queue(&self, inner_queue: CommandQueue) {
		self.0.send(inner_queue).await.unwrap_or_else(die)
	}

	/// Sends a (boxed) `Command` directly to the Bevy `World`, where they it be applied during
	/// the `Last` schedule.
	pub async fn send_single(&self, single: BoxedCommand) {
		self.send_queue(single.into()).await;
	}
}

#[derive(Component)]
pub(crate) struct CommandQueueReceiver(Receiver<CommandQueue>);

impl CommandQueueReceiver {
	pub(crate) fn new(receiver: Receiver<CommandQueue>) -> Self {
		Self(receiver)
	}

	fn enqueue_into(&self, world_queue: &mut WorldCommandQueue) -> Result<(), ()> {
		loop {
			match self.0.try_recv() {
				Ok(command_queue) => world_queue.0.push(command_queue),
				Err(TryRecvError::Closed) => break Err(()),
				Err(TryRecvError::Empty) => break Ok(()),
			}
		}
	}
}

#[derive(Resource)]
pub(crate) struct WorldCommandQueue(Vec<CommandQueue>);

impl WorldCommandQueue {
	pub(crate) fn drain(&mut self) -> Vec<CommandQueue> {
		self.0.drain(..).collect()
	}
}

impl Default for WorldCommandQueue {
	fn default() -> Self {
		Self(Vec::with_capacity(16))
	}
}

pub(crate) fn initialize_command_queue(mut commands: Commands) {
	commands.init_resource::<WorldCommandQueue>();
}

pub(crate) fn receive_commands(
	mut commands: Commands,
	receivers: Query<(Entity, &CommandQueueReceiver)>,
	mut queue: ResMut<WorldCommandQueue>,
) {
	for (id, receiver) in receivers.iter() {
		if receiver.enqueue_into(&mut queue).is_err() {
			commands.entity(id).despawn()
		}
	}
}

pub(crate) fn apply_commands(world: &mut World) {
	let commands = world.resource_mut::<WorldCommandQueue>().drain();
	if !commands.is_empty() {
		debug!("applying {} CommandQueues", commands.len());
	}

	for mut command in commands {
		command.apply(world);
	}
}

#[cfg(test)]
mod tests {
	use crate::util::insert;
	use crate::wait_for::StartWaitingFor;
	use crate::{AsyncEcsPlugin, AsyncEntity, AsyncWorld};
	use bevy::prelude::*;
	use bevy::tasks::AsyncComputeTaskPool;

	use super::*;

	#[derive(Component)]
	struct Marker;

	#[derive(Default, Clone, Component)]
	struct Counter(u8);

	#[test]
	fn smoke() {
		let mut app = App::new();
		app.add_plugins((MinimalPlugins, AsyncEcsPlugin));

		let async_world = AsyncWorld::from_world(app.world_mut());
		let operation_sender = async_world.sender();
		let (sender, receiver) = async_channel::bounded(1);
		let command = BoxedCommand::new(move |world: &mut World| {
			let id = world.spawn(Marker).id();
			sender.send_blocking(id).unwrap();
		});
		let debugged = format!("{:?}", command);

		AsyncComputeTaskPool::get()
			.spawn(async move { async_world.apply(command).await })
			.detach();

		let id = loop {
			match receiver.try_recv() {
				Ok(id) => break id,
				Err(_) => app.update(),
			}
		};
		app.update();

		assert!(app.world().entity(id).get::<Marker>().is_some());
		assert_eq!("BoxedCommand", debugged);
		let debugged = format!("{:?}", CommandQueueBuilder::new(operation_sender));
		assert_eq!(
			"CommandQueueBuilder { inner: \"[..]\", sender: CommandQueueSender(Sender { .. }) }",
			debugged
		);
	}

	#[test]
	fn queue() {
		let mut app = App::new();
		app.add_plugins((MinimalPlugins, AsyncEcsPlugin));

		let async_world = AsyncWorld::from_world(app.world_mut());
		let id = app.world_mut().spawn_empty().id();
		let (start_waiting_for, value_rx) = StartWaitingFor::<Counter>::component(id);

		let fut = async move {
			async_world
				.start_queue()
				.push(insert(id, Counter(3)))
				.push(start_waiting_for)
				.apply()
				.await;
		};
		AsyncComputeTaskPool::get().spawn(fut).detach();

		let counter = loop {
			match value_rx.try_recv() {
				Ok(value) => break value,
				Err(_) => app.update(),
			}
		};

		assert_eq!(3, counter.0);
	}

	#[test]
	fn sender() {
		let mut app = App::new();
		app.add_plugins((MinimalPlugins, AsyncEcsPlugin));

		let async_world = AsyncWorld::from_world(app.world_mut());
		let sender = async_world.sender();
		let entity = AsyncEntity::new(Entity::PLACEHOLDER, async_world.clone());
		let other_sender = entity.sender();
		assert_eq!(4, sender.0.sender_count());
		assert_eq!(4, other_sender.0.sender_count());
	}
}
