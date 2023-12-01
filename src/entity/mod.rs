pub(crate) mod reflect;

use crate::operation::OperationSender;
use crate::{AsyncOperation, CowStr};
use async_channel::Sender;
use bevy_core::Name;
use bevy_ecs::prelude::*;
use bevy_ecs::system::Command;
use bevy_hierarchy::DespawnRecursiveExt;
use bevy_reflect::prelude::*;
use reflect::ReflectOperation;
use std::any::TypeId;

pub(crate) use reflect::wait_for_reflect_components;
pub use reflect::AsyncComponent;

/// An `Entity`-related operation that can be applied to an `AsyncWorld`.
#[derive(Debug)]
#[non_exhaustive]
pub enum EntityOperation {
	/// Spawn an empty `Entity`. The spawned entity's ID will be sent into the `Sender`.
	SpawnEmpty(Sender<Entity>),
	/// Spawn an `Entity` with the given `Name`. The spawned entity's ID will be sent into the `Sender`.
	SpawnNamed(CowStr, Sender<Entity>),
	/// Despawn the given `Entity`.
	Despawn(Entity),
	/// Perform a `Reflect`-related operation.
	Reflect(ReflectOperation),
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

/// Represents an `Entity` that can be manipulated asynchronously.
///
/// The easiest way to get an `AsyncEntity` is with `AsyncWorld::spawn_empty()`.
///
/// Dropping the `AsyncEntity` **WILL NOT** despawn the corresponding entity in the synchronous world.
/// Use `AsyncEntity::despawn` to despawn an entity asynchronously.
#[derive(Debug)]
pub struct AsyncEntity {
	id: Entity,
	sender: OperationSender,
}

impl From<EntityOperation> for AsyncOperation {
	fn from(entity_op: EntityOperation) -> Self {
		Self::Entity(entity_op)
	}
}

impl AsyncEntity {
	/// Returns the underlying `Entity` being represented.
	pub fn id(&self) -> Entity {
		self.id
	}

	/// Returns a copy of the underlying `OperationSender`.
	pub fn sender(&self) -> OperationSender {
		self.sender.clone()
	}

	pub(crate) fn new(id: Entity, sender: OperationSender) -> Self {
		Self { id, sender }
	}

	pub(crate) async fn new_empty(sender: OperationSender) -> Self {
		let (id_sender, id_receiver) = async_channel::bounded(1);

		let operation = EntityOperation::SpawnEmpty(id_sender);
		sender.send(operation).await;

		let id = id_receiver.recv().await.expect("invariant broken");
		Self { id, sender }
	}

	pub(crate) async fn new_named(name: CowStr, sender: OperationSender) -> Self {
		let (id_sender, id_receiver) = async_channel::bounded(1);

		let operation = EntityOperation::SpawnNamed(name, id_sender);
		sender.send(operation).await;

		let id = id_receiver.recv().await.expect("invariant broken");
		Self { id, sender }
	}

	pub(crate) async fn new_bundle(bundle: Box<dyn Reflect>, sender: OperationSender) -> Self {
		let (id_sender, id_receiver) = async_channel::bounded(1);

		let operation = ReflectOperation::SpawnWithBundle(bundle, id_sender);
		sender.send(operation).await;

		let id = id_receiver.recv().await.expect("invariant broken");
		Self { id, sender }
	}

	/// Despawns the represented entity.
	pub async fn despawn(self) {
		self.sender.send(EntityOperation::Despawn(self.id)).await;
	}

	/// Adds a `Component` to the entity. This will overwrite any previous value(s) of the same component type.
	pub async fn insert_component<C: Component + Reflect>(&self, component: C) {
		let operation = ReflectOperation::InsertComponent(self.id, Box::new(component));
		self.sender.send(operation).await;
	}

	/// Adds a `Bundle` of components to the entity. This will overwrite any previous value(s) of
	/// the same component type.
	pub async fn insert_bundle<B: Bundle + Reflect>(&self, bundle: B) {
		let operation = ReflectOperation::InsertBundle(self.id, Box::new(bundle));
		self.sender.send(operation).await;
	}

	/// Removes a `Component` from the entity.
	pub async fn remove_component<C: Component + Reflect>(&self) {
		let operation = ReflectOperation::RemoveComponent(self.id, TypeId::of::<C>());
		self.sender.send(operation).await;
	}

	/// Removes a `Bundle` of components from the entity.
	pub async fn remove_bundle<B: Bundle + Reflect>(&self) {
		let operation = ReflectOperation::RemoveBundle(self.id, TypeId::of::<B>());
		self.sender.send(operation).await;
	}

	/// Start waiting for the `Component` of a given type. Returns an `AsyncComponent` which can be further
	/// waited to receive the value of the component.
	///
	/// `AsyncComponent::wait_for().await` is equivalent to
	/// `AsyncComponent::start_waiting_for().await.wait().await`.
	pub async fn start_waiting_for<C: Component + FromReflect>(&self) -> AsyncComponent<C> {
		let (sender, receiver) = async_channel::bounded(1);
		let operation = ReflectOperation::WaitForComponent(self.id, TypeId::of::<C>(), sender);
		self.sender.send(operation).await;
		AsyncComponent::new(receiver)
	}

	/// Wait for the `Component` of a given type. Returns the value of the component, once it exists
	/// on the represented entity.
	///
	/// `AsyncComponent::wait_for().await` is equivalent to
	/// `AsyncComponent::start_waiting_for().await.wait().await`.
	pub async fn wait_for<C: Component + FromReflect>(&self) -> C {
		self.start_waiting_for().await.wait().await
	}

	/// Insert the given `Component` of type `I` onto the entity, then immediately wait for a
	/// component of type `WR` to be added to the entity. After one is received, this will then
	/// remove the component of type `WR`.
	pub async fn insert_wait_remove<I: Component + Reflect, WR: Component + FromReflect>(
		&self,
		component: I,
	) -> WR {
		self.insert_component(component).await;
		let wr = self.wait_for::<WR>().await;
		self.remove_component::<WR>().await;
		wr
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
				sender.send(entity.id()).await.unwrap();
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
				Ok(_) => break,
				Err(_) => app.update(),
			}
		}
		app.update();

		assert!(app.world.get_entity(id).is_none());
	}
}
