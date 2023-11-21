mod reflect;

use crate::{AsyncOperation, CowStr, OperationSender};
use async_channel::{Receiver, Sender};
use bevy::ecs::system::Command;
use bevy::prelude::*;
use reflect::ReflectOperation;
use std::any::TypeId;
use std::marker::PhantomData;

pub use reflect::wait_for_reflect_components;

pub enum EntityOperation {
	SpawnEmpty(Sender<Entity>),
	SpawnNamed(CowStr, Sender<Entity>),
	Despawn(Entity),
	Reflect(ReflectOperation),
}

impl From<EntityOperation> for AsyncOperation {
	fn from(entity_op: EntityOperation) -> Self {
		Self::Entity(entity_op)
	}
}

impl Command for EntityOperation {
	fn apply(self, world: &mut World) {
		match self {
			EntityOperation::SpawnEmpty(sender) => {
				let id = world.spawn_empty().id();
				sender.try_send(id).expect("invariant broken");
			}
			EntityOperation::SpawnNamed(name, sender) => {
				let id = world.spawn(Name::new(name)).id();
				sender.try_send(id).expect("invariant broken");
			}
			EntityOperation::Despawn(id) => world.entity_mut(id).despawn_recursive(),
			EntityOperation::Reflect(reflect) => reflect.apply(world),
		}
	}
}

pub struct AsyncEntity {
	id: Entity,
	sender: OperationSender,
}

impl AsyncEntity {
	pub fn id(&self) -> Entity {
		self.id
	}

	pub(super) fn new(id: Entity, sender: OperationSender) -> Self {
		Self { id, sender }
	}

	pub(super) async fn new_empty(sender: OperationSender) -> Self {
		let (id_sender, id_receiver) = async_channel::bounded(1);

		let operation = EntityOperation::SpawnEmpty(id_sender);
		sender.send(operation).await;

		let id = id_receiver.recv().await.expect("invariant broken");
		Self { id, sender }
	}

	pub(super) async fn new_named(name: CowStr, sender: OperationSender) -> Self {
		let (id_sender, id_receiver) = async_channel::bounded(1);

		let operation = EntityOperation::SpawnNamed(name, id_sender);
		sender.send(operation).await;

		let id = id_receiver.recv().await.expect("invariant broken");
		Self { id, sender }
	}

	pub(super) async fn new_bundle(bundle: Box<dyn Reflect>, sender: OperationSender) -> Self {
		let (id_sender, id_receiver) = async_channel::bounded(1);

		let operation = ReflectOperation::SpawnWithBundle(bundle, id_sender);
		sender.send(operation).await;

		let id = id_receiver.recv().await.expect("invariant broken");
		Self { id, sender }
	}

	pub async fn despawn(self) {
		self.sender.send(EntityOperation::Despawn(self.id)).await;
	}

	pub async fn insert_component<C: Component + Reflect>(&self, component: C) {
		let operation = ReflectOperation::InsertComponent(self.id, Box::new(component));
		self.sender.send(operation).await;
	}

	pub async fn insert_bundle<B: Bundle + Reflect>(&self, bundle: B) {
		let operation = ReflectOperation::InsertBundle(self.id, Box::new(bundle));
		self.sender.send(operation).await;
	}

	pub async fn remove_component<C: Component + Reflect>(&self) {
		let operation = ReflectOperation::RemoveComponent(self.id, TypeId::of::<C>());
		self.sender.send(operation).await;
	}

	pub async fn remove_bundle<B: Bundle + Reflect>(&self) {
		let operation = ReflectOperation::RemoveBundle(self.id, TypeId::of::<B>());
		self.sender.send(operation).await;
	}

	pub async fn start_waiting_for<C: Component + FromReflect>(&self) -> AsyncComponent<C> {
		let (sender, receiver) = async_channel::bounded(1);
		let operation = ReflectOperation::WaitForComponent(self.id, TypeId::of::<C>(), sender);
		self.sender.send(operation).await;
		AsyncComponent(receiver, PhantomData)
	}

	pub async fn wait_for<C: Component + FromReflect>(&self) -> C {
		self.start_waiting_for().await.wait().await
	}
}

pub struct AsyncComponent<T: Component + FromReflect>(Receiver<Box<dyn Reflect>>, PhantomData<T>);

impl<C: Component + FromReflect> AsyncComponent<C> {
	pub async fn wait(self) -> C {
		let boxed_dynamic = self.0.recv().await.expect("invariant broken");
		C::take_from_reflect(boxed_dynamic).expect("invariant broken")
	}
}

#[cfg(test)]
mod tests {
	use crate::{AsyncEcsPlugin, AsyncWorld};
	use bevy::prelude::*;
	use futures_lite::future;

	#[test]
	fn smoke() {
		let mut app = App::new();
		app.add_plugins((MinimalPlugins, AsyncEcsPlugin));

		let (sender, receiver) = async_channel::bounded(1);
		let async_world = AsyncWorld::from_world(&mut app.world);

		std::thread::spawn(move || {
			future::block_on(async move {
				let entity = async_world.spawn_empty().await;
				sender.send(entity.id).await.unwrap();
			});
		});

		let id = loop {
			match receiver.try_recv() {
				Ok(id) => break id,
				Err(_) => app.update(),
			}
		};

		assert!(app.world.get_entity(id).is_some());
	}

	#[test]
	fn named() {
		let mut app = App::new();
		app.add_plugins((MinimalPlugins, AsyncEcsPlugin));

		let (sender, receiver) = async_channel::bounded(1);
		let async_world = AsyncWorld::from_world(&mut app.world);

		std::thread::spawn(move || {
			future::block_on(async move {
				let entity = async_world.spawn_named("lol").await;
				sender.send(entity.id).await.unwrap();
			});
		});

		let id = loop {
			match receiver.try_recv() {
				Ok(id) => break id,
				Err(_) => app.update(),
			}
		};
		app.update();

		let name = app.world.entity(id).get::<Name>().unwrap();
		assert_eq!("lol", name.as_str());
	}

	#[test]
	fn despawn() {
		let mut app = App::new();
		app.add_plugins((MinimalPlugins, AsyncEcsPlugin));

		let (sender, receiver) = async_channel::bounded(1);
		let async_world = AsyncWorld::from_world(&mut app.world);
		let id = app.world.spawn_empty().id();

		std::thread::spawn(move || {
			future::block_on(async move {
				let entity = async_world.entity(id);
				entity.despawn().await;
				sender.send(()).await.unwrap();
			});
		});

		loop {
			match receiver.try_recv() {
				Ok(value) => break,
				Err(_) => app.update(),
			}
		}
		app.update();

		assert!(app.world.get_entity(id).is_none());
	}
}
