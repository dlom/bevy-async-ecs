use crate::command::BoxedCommand;
use crate::resource::ResourceOperation;
use crate::{
	AsyncEntity, AsyncIOSystem, AsyncResource, AsyncSystem, OperationReceiver, OperationSender,
};
use bevy::ecs::system::Command;
use bevy::prelude::*;
use std::any::TypeId;
use std::borrow::Cow;

#[derive(Clone)]
pub struct AsyncWorld(OperationSender);

impl AsyncWorld {
	pub async fn apply_command<C: Command>(&self, command: C) {
		self.0.send(BoxedCommand::new(command)).await;
	}

	pub async fn register_system<M>(&self, system: impl IntoSystem<(), (), M>) -> AsyncSystem {
		let system = Box::new(IntoSystem::into_system(system));
		AsyncSystem::new(system, self.0.clone()).await
	}

	pub async fn register_io_system<I: Send + 'static, O: Send + 'static, M>(
		&self,
		system: impl IntoSystem<I, O, M>,
	) -> AsyncIOSystem<I, O> {
		AsyncIOSystem::new(system, self.0.clone()).await
	}

	pub fn entity(&self, id: Entity) -> AsyncEntity {
		AsyncEntity::new(id, self.0.clone())
	}

	pub async fn spawn_empty(&self) -> AsyncEntity {
		AsyncEntity::new_empty(self.0.clone()).await
	}

	pub async fn spawn_named(&self, name: impl Into<Cow<'static, str>>) -> AsyncEntity {
		AsyncEntity::new_named(name.into(), self.0.clone()).await
	}

	pub async fn spawn<B: Bundle + Reflect>(&self, bundle: B) -> AsyncEntity {
		AsyncEntity::new_bundle(Box::new(bundle), self.0.clone()).await
	}

	pub async fn insert_resource<R: Resource + Reflect>(&self, resource: R) {
		let operation = ResourceOperation::Insert(Box::new(resource));
		self.0.send(operation).await;
	}

	pub async fn remove_resource<R: Resource + Reflect>(&self) {
		let operation = ResourceOperation::Remove(TypeId::of::<R>());
		self.0.send(operation).await;
	}

	pub async fn start_waiting_for_resource<R: Resource + FromReflect>(&self) -> AsyncResource<R> {
		let (sender, receiver) = async_channel::bounded(1);
		let operation = ResourceOperation::WaitFor(TypeId::of::<R>(), sender);
		self.0.send(operation).await;
		AsyncResource::new(receiver)
	}

	pub async fn wait_for_resource<R: Resource + FromReflect>(&self) -> R {
		self.start_waiting_for_resource().await.wait().await
	}
}

impl FromWorld for AsyncWorld {
	fn from_world(world: &mut World) -> Self {
		let (sender, receiver) = async_channel::unbounded();
		world.spawn((OperationReceiver(receiver), Name::new("AsyncReceiver")));
		Self(OperationSender(sender))
	}
}
