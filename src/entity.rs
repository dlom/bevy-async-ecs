use crate::command::CommandQueueSender;
use crate::util::{insert, remove};
use crate::wait_for::StartWaitingFor;
use crate::world::AsyncWorld;
use crate::{die, recv};
use async_channel::{Receiver, Sender};
use bevy_ecs::prelude::*;
use bevy_ecs::world::Command;
use bevy_hierarchy::DespawnRecursive;
use std::fmt;

/// Represents an `Entity` that can be manipulated asynchronously.
///
/// The easiest way to get an `AsyncEntity` is with `AsyncWorld::spawn_empty()`.
///
/// Dropping the `AsyncEntity` **WILL NOT** despawn the corresponding entity in the synchronous world.
/// Use `AsyncEntity::despawn` to despawn an entity asynchronously.
#[derive(Debug)]
pub struct AsyncEntity {
	id: Entity,
	world: AsyncWorld,
}

impl AsyncEntity {
	pub(crate) fn new(id: Entity, world: AsyncWorld) -> Self {
		Self { id, world }
	}

	/// Returns the underlying `Entity` being represented.
	pub fn id(&self) -> Entity {
		self.id
	}

	/// Returns a copy of the underlying `CommandQueueSender`.
	pub fn sender(&self) -> CommandQueueSender {
		self.world.sender()
	}

	/// Recursively despawns the represented entity.
	pub async fn despawn(self) {
		self.world.apply(DespawnRecursive { entity: self.id }).await;
	}

	/// Adds a `Bundle` of components to the entity. This will overwrite any previous value(s) of
	/// the same component type.
	pub async fn insert<B: Bundle>(&self, bundle: B) {
		self.world.apply(insert(self.id, bundle)).await;
	}

	/// Removes a `Bundle` of components from the entity.
	pub async fn remove<B: Bundle>(&self) {
		self.world.apply(remove::<B>(self.id)).await;
	}

	/// Start waiting for the `Component` of a given type. Returns an `AsyncComponent` which can be further
	/// waited to receive the value of the component.
	///
	/// `AsyncComponent::wait_for().await` is equivalent to
	/// `AsyncComponent::start_waiting_for().await.wait().await`.
	pub async fn start_waiting_for<C: Component + Clone>(&self) -> AsyncComponent<C> {
		let (start_waiting_for, rx) = StartWaitingFor::component(self.id);
		self.world.apply(start_waiting_for).await;
		AsyncComponent(rx)
	}

	/// Wait for the `Component` of a given type. Returns the value of the component, once it exists
	/// on the represented entity.
	///
	/// `AsyncComponent::wait_for().await` is equivalent to
	/// `AsyncComponent::start_waiting_for().await.wait().await`.
	pub async fn wait_for<C: Component + Clone>(&self) -> C {
		self.start_waiting_for().await.wait().await
	}

	/// Insert the given `Component` of type `I` onto the entity, then immediately wait for a
	/// component of type `WR` to be added to the entity. After one is received, this will then
	/// remove the component of type `WR`.
	pub async fn insert_wait_remove<I: Component, WR: Component + Clone>(
		&self,
		component: I,
	) -> WR {
		self.insert(component).await;
		let wr = self.wait_for::<WR>().await;
		self.remove::<WR>().await;
		wr
	}
}

/// Represents a `Component` being retrieved.
///
/// The easiest way to get an `AsyncComponent` is with `AsyncEntity::start_waiting_for()`.
pub struct AsyncComponent<C: Component>(Receiver<C>);

impl<C: Component> fmt::Debug for AsyncComponent<C> {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "AsyncComponent(..)")
	}
}

impl<C: Component> AsyncComponent<C> {
	/// Wait for the `Component` to exist, and retrieve its value.
	pub async fn wait(self) -> C {
		recv(self.0).await
	}
}

pub(crate) struct SpawnAndSendId<B> {
	bundle: B,
	sender: Sender<Entity>,
}

impl SpawnAndSendId<()> {
	pub(crate) fn new_empty() -> (Self, Receiver<Entity>) {
		let (sender, receiver) = async_channel::bounded(1);
		(Self { bundle: (), sender }, receiver)
	}
}

impl<B: Bundle> SpawnAndSendId<B> {
	pub(crate) fn new(bundle: B) -> (Self, Receiver<Entity>) {
		let (sender, receiver) = async_channel::bounded(1);
		(Self { bundle, sender }, receiver)
	}
}

impl<B: Bundle> Command for SpawnAndSendId<B> {
	fn apply(self, world: &mut World) {
		let id = world.spawn(self.bundle).id();
		self.sender.try_send(id).unwrap_or_else(die);
	}
}

#[cfg(test)]
mod tests {
	use crate::{AsyncEcsPlugin, AsyncWorld};
	use bevy::prelude::*;
	use bevy::tasks::AsyncComputeTaskPool;

	#[derive(Default, Clone, Component)]
	struct Translation(u8, u8);

	#[derive(Default, Clone, Component)]
	struct Scale(u8, u8);

	#[derive(Default, Clone, Bundle)]
	struct Transform {
		translation: Translation,
		scale: Scale,
	}

	#[test]
	fn smoke() {
		let mut app = App::new();
		app.add_plugins((MinimalPlugins, AsyncEcsPlugin));

		let (sender, receiver) = async_channel::bounded(1);
		let async_world = AsyncWorld::from_world(app.world_mut());

		AsyncComputeTaskPool::get()
			.spawn(async move {
				let entity = async_world.spawn_empty().await;
				sender.send(entity.id()).await.unwrap();
			})
			.detach();

		let id = loop {
			match receiver.try_recv() {
				Ok(id) => break id,
				Err(_) => app.update(),
			}
		};

		assert!(app.world().get_entity(id).is_some());
	}

	#[test]
	fn named() {
		let mut app = App::new();
		app.add_plugins((MinimalPlugins, AsyncEcsPlugin));

		let (sender, receiver) = async_channel::bounded(1);
		let async_world = AsyncWorld::from_world(app.world_mut());

		AsyncComputeTaskPool::get()
			.spawn(async move {
				let entity = async_world.spawn_named("lol").await;
				sender.send(entity.id).await.unwrap();
			})
			.detach();

		let id = loop {
			match receiver.try_recv() {
				Ok(id) => break id,
				Err(_) => app.update(),
			}
		};

		let name = app.world().entity(id).get::<Name>().unwrap();
		assert_eq!("lol", name.as_str());
	}

	#[test]
	fn despawn() {
		let mut app = App::new();
		app.add_plugins((MinimalPlugins, AsyncEcsPlugin));

		let (sender, receiver) = async_channel::bounded(1);
		let async_world = AsyncWorld::from_world(app.world_mut());
		let id = app.world_mut().spawn_empty().id();

		AsyncComputeTaskPool::get()
			.spawn(async move {
				let entity = async_world.entity(id);
				entity.despawn().await;
				sender.send(()).await.unwrap();
			})
			.detach();

		loop {
			match receiver.try_recv() {
				Ok(_) => break,
				Err(_) => app.update(),
			}
		}

		assert!(app.world().get_entity(id).is_none());
	}

	#[test]
	fn spawn() {
		let mut app = App::new();
		app.add_plugins((MinimalPlugins, AsyncEcsPlugin));

		let (sender, receiver) = async_channel::bounded(1);
		let async_world = AsyncWorld::from_world(app.world_mut());

		AsyncComputeTaskPool::get()
			.spawn(async move {
				let entity = async_world
					.spawn(Transform {
						translation: Translation(2, 3),
						scale: Scale(1, 1),
					})
					.await;
				sender.send(entity.id).await.unwrap();
			})
			.detach();

		let id = loop {
			match receiver.try_recv() {
				Ok(id) => break id,
				Err(_) => app.update(),
			}
		};

		let translation = app.world().get::<Translation>(id).unwrap();
		assert_eq!(2, translation.0);
		assert_eq!(3, translation.1);
		let scale = app.world().get::<Scale>(id).unwrap();
		assert_eq!(1, scale.0);
		assert_eq!(1, scale.1);
	}

	#[test]
	fn insert() {
		let mut app = App::new();
		app.add_plugins((MinimalPlugins, AsyncEcsPlugin));

		let (sender, receiver) = async_channel::bounded(1);
		let async_world = AsyncWorld::from_world(app.world_mut());

		AsyncComputeTaskPool::get()
			.spawn(async move {
				let entity = async_world.spawn_empty().await;
				sender.send(entity.id).await.unwrap();
				entity
					.insert(Transform {
						translation: Translation(2, 3),
						scale: Scale(1, 1),
					})
					.await;
			})
			.detach();

		let id = loop {
			match receiver.try_recv() {
				Ok(id) => break id,
				Err(_) => app.update(),
			}
		};
		app.update();

		let translation = app.world().get::<Translation>(id).unwrap();
		assert_eq!(2, translation.0);
		assert_eq!(3, translation.1);
		let scale = app.world().get::<Scale>(id).unwrap();
		assert_eq!(1, scale.0);
		assert_eq!(1, scale.1);
	}

	#[test]
	fn remove() {
		let mut app = App::new();
		app.add_plugins((MinimalPlugins, AsyncEcsPlugin));

		let async_world = AsyncWorld::from_world(app.world_mut());
		let id = app
			.world_mut()
			.spawn(Transform {
				translation: Translation(3, 4),
				scale: Scale(1, 1),
			})
			.id();

		AsyncComputeTaskPool::get()
			.spawn(async move {
				async_world.entity(id).remove::<Transform>().await;
			})
			.detach();
		app.update();

		assert!(app.world().get::<Translation>(id).is_none());
		assert!(app.world().get::<Scale>(id).is_none());
	}

	#[test]
	fn insert_wait_remove() {
		let mut app = App::new();
		app.add_plugins((MinimalPlugins, AsyncEcsPlugin));

		let (value_tx, value_rx) = async_channel::bounded(1);
		let async_world = AsyncWorld::from_world(app.world_mut());
		let id = app.world_mut().spawn_empty().id();

		AsyncComputeTaskPool::get()
			.spawn(async move {
				let scale: Scale = async_world.entity(id).insert_wait_remove(Scale(6, 7)).await;
				value_tx.send(scale).await.unwrap();
			})
			.detach();

		let value = loop {
			match value_rx.try_recv() {
				Ok(value) => break value,
				Err(_) => app.update(),
			}
		};
		app.update();

		assert_eq!(6, value.0);
		assert_eq!(7, value.1);
		assert!(app.world().entity(id).get::<Scale>().is_none());
	}
}
