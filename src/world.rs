use crate::command::{BoxedCommand, CommandQueueBuilder, CommandQueueReceiver, CommandQueueSender};
use crate::entity::{AsyncEntity, SpawnAndSendId};
use crate::system::{AsyncIOSystem, AsyncSystem};
use crate::util::{insert_resource, remove_resource, trigger_event};
use crate::wait_for::StartWaitingFor;
use crate::{die, recv, CowStr};
use async_channel::Receiver;
use bevy_ecs::prelude::*;
use bevy_ecs::system::RunSystemOnce;
use std::fmt;

/// Exposes asynchronous access to the Bevy ECS `World`.
///
/// The easiest way to get an `AsyncWorld` is with `AsyncWorld::from_world()`.
///
/// ## Commands
/// Apply any `Command` asynchronously with `AsyncWorld::apply_command`.
///
/// ## Systems
/// Just like their synchronous variants, asynchronous `System`s must be registered
/// before they can be used. Systems can optionally accept and return values asynchronously
/// if they are registered with `AsyncWorld::register_io_system`.
///
/// ## Entities
/// Spawn entities with the `AsyncWorld::spawn_*` family.
///
/// ## Resources
/// Insert, remove, and wait for resources to exist.
#[derive(Clone, Debug)]
pub struct AsyncWorld(CommandQueueSender);

impl AsyncWorld {
	/// Returns a copy of the underlying `CommandQueueSender`.
	pub fn sender(&self) -> CommandQueueSender {
		self.0.clone()
	}

	/// Applies the given `Command` to the world.
	pub async fn apply<C: Command>(&self, command: C) {
		self.0.send_single(BoxedCommand::new(command)).await
	}

	/// Starts building a `CommandQueue`.
	pub fn start_queue(&self) -> CommandQueueBuilder {
		CommandQueueBuilder::new(self.sender())
	}

	/// Run a [`System`] once.
	pub async fn run_system<M>(self, system: impl IntoSystem<(), (), M> + Send + 'static) {
		self.apply(|world: &mut World| {
			_ = world.run_system_once(system);
		})
		.await
	}

	/// Registers a `System` and returns an `AsyncSystem` that can be used to run the system on demand.
	pub async fn register_system<M>(
		&self,
		system: impl IntoSystem<(), (), M> + Send,
	) -> AsyncSystem {
		let system = Box::new(IntoSystem::into_system(system));
		AsyncSystem::new(system, self.clone()).await
	}

	/// Registers a `System` and returns an `AsyncIOSystem` that can be used to run the system on demand
	/// while supplying an input value and receiving an output value.
	pub async fn register_io_system<I: Send + 'static, O: Send + 'static, M>(
		&self,
		system: impl IntoSystem<In<I>, O, M> + Send,
	) -> AsyncIOSystem<I, O> {
		AsyncIOSystem::new(system, self.clone()).await
	}

	/// Constructs an `AsyncEntity` for the given `Entity`. If the entity does not exist, any operation
	/// performed on it will panic.
	pub fn entity(&self, id: Entity) -> AsyncEntity {
		AsyncEntity::new(id, self.clone())
	}

	/// Spawns a new `Entity` and returns an `AsyncEntity` that represents it, which can be used
	/// to further manipulate the entity.
	pub async fn spawn_empty(&self) -> AsyncEntity {
		let (command, receiver) = SpawnAndSendId::new_empty();
		self.apply(command).await;
		let id = recv(receiver).await;
		AsyncEntity::new(id, self.clone())
	}

	/// Spawns a new `Entity` with the given `Bundle` and returns an `AsyncEntity` that represents it,
	/// which can be used to further manipulate the entity.
	pub async fn spawn<B: Bundle>(&self, bundle: B) -> AsyncEntity {
		let (command, receiver) = SpawnAndSendId::new(bundle);
		self.apply(command).await;
		let id = recv(receiver).await;
		AsyncEntity::new(id, self.clone())
	}

	/// Spawns a new `Entity` and returns an `AsyncEntity` that represents it, which can be used
	/// to further manipulate the entity. This function attaches a bevy `Name` component with the given
	/// value.
	pub async fn spawn_named(&self, name: impl Into<CowStr> + Send) -> AsyncEntity {
		self.spawn(Name::new(name)).await
	}

	/// Inserts a new resource or updates an existing resource with the given value.
	pub async fn insert_resource<R: Resource>(&self, resource: R) {
		self.apply(insert_resource(resource)).await;
	}

	/// Removes the resource of a given type, if it exists.
	pub async fn remove_resource<R: Resource>(&self) {
		self.apply(remove_resource::<R>()).await;
	}

	/// Start waiting for the `Resource` of a given type. Returns an `AsyncResource` which can be further
	/// waited to receive the value of the resource.
	///
	/// `AsyncWorld::wait_for_resource().await` is equivalent to
	/// `AsyncWorld::start_waiting_for_resource().await.wait().await`.
	pub async fn start_waiting_for_resource<R: Resource + Clone>(&self) -> AsyncResource<R> {
		let (start_waiting_for, rx) = StartWaitingFor::resource();
		self.apply(start_waiting_for).await;
		AsyncResource(rx)
	}

	/// Wait for the `Resource` of a given type. Returns the value of the resource, once it exists.
	///
	/// `AsyncWorld::wait_for_resource().await` is equivalent to
	/// `AsyncWorld::start_waiting_for_resource().await.wait().await`.
	pub async fn wait_for_resource<R: Resource + Clone>(&self) -> R {
		self.start_waiting_for_resource().await.wait().await
	}

	/// Send a `Message` to the bevy world.
	pub async fn send_message<M: Message>(&self, message: M) {
		self.apply(WriteMessage(message)).await;
	}

	/// Start listening for `Message`s coming from the main bevy world.
	/// Returns an `AsyncMessages` which can be further waited to receive these messages.
	///
	/// `AsyncWorld::wait_for_message().await` is equivalent to
	/// `AsyncWorld::start_waiting_for_messages().await.wait().await`.
	pub async fn start_waiting_for_messages<M: Message + Clone>(&self) -> AsyncMessages<M> {
		let (start_waiting_for, rx) = StartWaitingFor::messages();
		self.apply(start_waiting_for).await;
		AsyncMessages(rx)
	}

	/// Wait for the `Message` of a given type. Returns the value of the message, once it is received.
	///
	/// `AsyncWorld::wait_for_message().await` is equivalent to
	/// `AsyncWorld::start_waiting_for_messages().await.wait().await`.
	pub async fn wait_for_message<M: Message + Clone>(&self) -> M {
		self.start_waiting_for_messages().await.wait().await
	}

	/// Triggers the given [`Event`], which will run any [`Observer`]s watching for it.
	pub async fn trigger<'a, T: Default, E: Event<Trigger<'a> = T> + Send + Sync + 'static>(&self, event: E) {
		self.apply(trigger_event(event)).await;
	}
}

impl From<CommandQueueSender> for AsyncWorld {
	fn from(sender: CommandQueueSender) -> Self {
		Self(sender)
	}
}

impl FromWorld for AsyncWorld {
	fn from_world(world: &mut World) -> Self {
		let (sender, receiver) = async_channel::unbounded();
		world.spawn((
			CommandQueueReceiver::new(receiver),
			Name::new("CommandQueueReceiver"),
		));
		CommandQueueSender::new(sender).into()
	}
}

/// Represents a `Resource` being retrieved.
///
/// The easiest way to get an `AsyncResource` is with `AsyncWorld::start_waiting_for_resource()`.
pub struct AsyncResource<R: Resource>(Receiver<R>);

impl<R: Resource> fmt::Debug for AsyncResource<R> {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "AsyncResource(..)")
	}
}

impl<R: Resource> AsyncResource<R> {
	/// Wait for the `Resource` to exist, and retrieve its value.
	pub async fn wait(self) -> R {
		recv(self.0).await
	}
}

struct WriteMessage<M: Message>(M);

impl<M: Message> Command for WriteMessage<M> {
	fn apply(self, world: &mut World) {
		world
			.write_message(self.0)
			.ok_or("failed to write message")
			.unwrap_or_else(die);
	}
}

/// Represents Bevy `Message`s being received asynchronously
///
/// The easiest way to get an `AsyncMessages` is with `AsyncWorld::start_waiting_for_messages()`.
pub struct AsyncMessages<M: Message>(Receiver<M>);

impl<M: Message> fmt::Debug for AsyncMessages<M> {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "AsyncMessages(..)")
	}
}

impl<M: Message> AsyncMessages<M> {
	/// Wait for a `Message` to be received from the vanilla Bevy world. This function can be called repeatedly
	/// to get more messages as they are received.
	pub async fn wait(&self) -> M {
		recv(self.0.clone()).await
	}
}
