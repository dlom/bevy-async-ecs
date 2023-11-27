use crate::command::BoxedCommand;
use crate::operations::AsyncOperation;
use crate::resource::ResourceOperation;
use crate::{
	AsyncEntity, AsyncIOSystem, AsyncResource, AsyncSystem, OperationReceiver, OperationSender,
};
use bevy::ecs::system::Command;
use bevy::prelude::*;
use std::any::TypeId;
use std::borrow::Cow;

/// Exposes asynchronous access to the Bevy ECS `World`.
///
/// # Commands
/// Apply any `Command` asynchronously with `AsyncWorld::apply_command`.
///
/// # Systems
/// Just like their synchronous variants, asynchronous `System`s must be registered
/// before they can be used. Systems can optionally accept and return values asynchronously
/// if they are registered with `AsyncWorld::register_io_system`.
///
/// # Entities
/// Spawn entities with the `AsyncWorld::spawn_*` family.
///
/// # Resources
/// Insert, remove, and wait for resources to exist.
#[derive(Clone)]
pub struct AsyncWorld(OperationSender);

impl AsyncWorld {
	/// Applies the given `Command` to the world.
	pub async fn apply_command<C: Command>(&self, command: C) {
		self.0.send(BoxedCommand::new(command)).await;
	}

	/// Applies the given `Operation` to the world.
	pub async fn apply_operation(&self, operation: AsyncOperation) {
		self.0.send(operation).await;
	}

	/// Registers a `System` and returns an `AsyncSystem` that can be used to run the system on demand.
	pub async fn register_system<M>(&self, system: impl IntoSystem<(), (), M>) -> AsyncSystem {
		let system = Box::new(IntoSystem::into_system(system));
		AsyncSystem::new(system, self.0.clone()).await
	}

	/// Registers a `System` and returns an `AsyncIOSystem` that can be used to run the system on demand
	/// while supplying an input value and receiving an output value.
	pub async fn register_io_system<I: Send + 'static, O: Send + 'static, M>(
		&self,
		system: impl IntoSystem<I, O, M>,
	) -> AsyncIOSystem<I, O> {
		AsyncIOSystem::new(system, self.0.clone()).await
	}

	/// Constructs an `AsyncEntity` for the given `Entity`. If the entity does not exist, any operation
	/// performed on it will panic.
	pub fn entity(&self, id: Entity) -> AsyncEntity {
		AsyncEntity::new(id, self.0.clone())
	}

	/// Spawns a new `Entity` and returns an `AsyncEntity` that represents it, which can be used
	/// to further manipulate the entity.
	pub async fn spawn_empty(&self) -> AsyncEntity {
		AsyncEntity::new_empty(self.0.clone()).await
	}

	/// Spawns a new `Entity` and returns an `AsyncEntity` that represents it, which can be used
	/// to further manipulate the entity. This function attaches a bevy `Name` component with the given
	/// value.
	pub async fn spawn_named(&self, name: impl Into<Cow<'static, str>>) -> AsyncEntity {
		AsyncEntity::new_named(name.into(), self.0.clone()).await
	}

	/// Spawns a new `Entity` with the given `Bundle` and returns an `AsyncEntity` that represents it,
	/// which can be used to further manipulate the entity.
	pub async fn spawn<B: Bundle + Reflect>(&self, bundle: B) -> AsyncEntity {
		AsyncEntity::new_bundle(Box::new(bundle), self.0.clone()).await
	}

	/// Inserts a new resource or updates an existing resource with the given value.
	pub async fn insert_resource<R: Resource + Reflect>(&self, resource: R) {
		let operation = ResourceOperation::Insert(Box::new(resource));
		self.0.send(operation).await;
	}

	/// Removes the resource of a given type, if it exists.
	pub async fn remove_resource<R: Resource + Reflect>(&self) {
		let operation = ResourceOperation::Remove(TypeId::of::<R>());
		self.0.send(operation).await;
	}

	/// Start waiting for the `Resource` of a given type. Returns an `AsyncResource` which can be further
	/// waited to receive the value of the resource.
	///
	/// `AsyncWorld::wait_for_resource().await` is equivalent to
	/// `AsyncWorld::start_waiting_for_resource().await.wait().await`.
	pub async fn start_waiting_for_resource<R: Resource + FromReflect>(&self) -> AsyncResource<R> {
		let (sender, receiver) = async_channel::bounded(1);
		let operation = ResourceOperation::WaitFor(TypeId::of::<R>(), sender);
		self.0.send(operation).await;
		AsyncResource::new(receiver)
	}

	/// Wait for the `Resource` of a given type. Returns the value of the resource, once it exists.
	///
	/// `AsyncWorld::wait_for_resource().await` is equivalent to
	/// `AsyncWorld::start_waiting_for_resource().await.wait().await`.
	pub async fn wait_for_resource<R: Resource + FromReflect>(&self) -> R {
		self.start_waiting_for_resource().await.wait().await
	}
}

impl FromWorld for AsyncWorld {
	fn from_world(world: &mut World) -> Self {
		let (sender, receiver) = async_channel::unbounded();
		world.spawn((OperationReceiver(receiver), Name::new("OperationReceiver")));
		Self(OperationSender(sender))
	}
}
