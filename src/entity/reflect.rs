use super::EntityOperation;
use crate::AsyncOperation;
use async_channel::{Receiver, Sender};
use bevy_ecs::prelude::*;
use bevy_ecs::reflect::ReflectBundle;
use bevy_ecs::system::Command;
use bevy_reflect::prelude::*;
use bevy_reflect::TypeRegistry;
use std::any::TypeId;
use std::marker::PhantomData;

/// A `Reflect`-related operation that can be applied to an `AsyncWorld`.
#[derive(Debug)]
#[non_exhaustive]
pub enum ReflectOperation {
	/// Insert a boxed `Component` into the given `Entity`.
	InsertComponent(Entity, Box<dyn Reflect>),
	/// Insert a boxed `Bundle` into the given `Entity`.
	InsertBundle(Entity, Box<dyn Reflect>),
	/// Remove a `Component` specified by `TypeId` from the given `Entity`.
	RemoveComponent(Entity, TypeId),
	/// Remove a `Bundle` specified by `TypeId` from the given `Entity`.
	RemoveBundle(Entity, TypeId),
	/// Spawn an entity with the given `Bundle`. The spawned entity's ID will be sent into the `Sender`.
	SpawnWithBundle(Box<dyn Reflect>, Sender<Entity>),
	/// Wait for the `Component` specified by the `TypeId` to exist on the given entity. As soon as
	/// the component exists (or if it already exists), the value of the component will be sent into
	/// the `Sender`.
	WaitForComponent(Entity, TypeId, Sender<Box<dyn Reflect>>),
}

impl Command for ReflectOperation {
	fn apply(self, world: &mut World) {
		world.resource_scope(|world, registry: Mut<AppTypeRegistry>| {
			let registry = registry.read();
			match self {
				ReflectOperation::InsertComponent(id, boxed) => {
					let mut entity = world.entity_mut(id);
					let reflect_component = get_reflect_component(&registry, (*boxed).type_id());
					reflect_component.apply_or_insert(&mut entity, boxed.as_reflect());
				}
				ReflectOperation::InsertBundle(id, boxed) => {
					let mut entity = world.entity_mut(id);
					let reflect_bundle = get_reflect_bundle(&registry, (*boxed).type_id());
					reflect_bundle.apply_or_insert(&mut entity, boxed.as_reflect(), &registry);
				}
				ReflectOperation::RemoveComponent(id, type_id) => {
					let mut entity = world.entity_mut(id);
					let reflect_component = get_reflect_component(&registry, type_id);
					reflect_component.remove(&mut entity);
				}
				ReflectOperation::RemoveBundle(id, type_id) => {
					let mut entity = world.entity_mut(id);
					let reflect_bundle = get_reflect_bundle(&registry, type_id);
					reflect_bundle.remove(&mut entity);
				}
				ReflectOperation::SpawnWithBundle(boxed, sender) => {
					let mut entity = world.spawn_empty();
					let reflect_bundle = get_reflect_bundle(&registry, (*boxed).type_id());
					reflect_bundle.apply_or_insert(&mut entity, boxed.as_reflect(), &registry);
					sender.try_send(entity.id()).expect("invariant broken");
				}
				ReflectOperation::WaitForComponent(id, type_id, sender) => {
					let reflect_component = get_reflect_component(&registry, type_id);
					if let Some(reflect) = reflect_component.reflect(world.entity(id)) {
						sender
							.try_send(reflect.clone_value())
							.expect("invariant broken");
					} else {
						world.entity_mut(id).insert(WaitingFor(type_id, sender));
					}
				}
			}
		});
	}
}

impl From<ReflectOperation> for AsyncOperation {
	fn from(reflect_op: ReflectOperation) -> Self {
		EntityOperation::Reflect(reflect_op).into()
	}
}

/// Represents a `Component` being retrieved.
#[derive(Debug)]
pub struct AsyncComponent<T: Component + FromReflect>(Receiver<Box<dyn Reflect>>, PhantomData<T>);

impl<C: Component + FromReflect> AsyncComponent<C> {
	pub(crate) fn new(receiver: Receiver<Box<dyn Reflect>>) -> Self {
		Self(receiver, PhantomData)
	}

	/// Wait for the `Component` to exist, and retrieve its value.
	pub async fn wait(self) -> C {
		let boxed_dynamic = self.0.recv().await.expect("invariant broken");
		C::take_from_reflect(boxed_dynamic).expect("invariant broken")
	}
}

#[derive(Component)]
pub(crate) struct WaitingFor(TypeId, Sender<Box<dyn Reflect>>);

pub(crate) fn wait_for_reflect_components(
	mut commands: Commands,
	query: Query<(Entity, &WaitingFor)>,
	registry: Res<AppTypeRegistry>,
	world: &World,
) {
	let registry = registry.read();
	for (id, WaitingFor(type_id, sender)) in query.iter() {
		let reflect_component = get_reflect_component(&registry, *type_id);
		if let Some(reflect) = reflect_component.reflect(world.entity(id)) {
			sender
				.try_send(reflect.clone_value())
				.expect("invariant broken");
			commands.entity(id).remove::<WaitingFor>();
		}
	}
}

fn get_reflect_component(registry: &TypeRegistry, type_id: TypeId) -> &ReflectComponent {
	let type_registration = registry.get(type_id).expect("reflect type not registered");
	type_registration
		.data::<ReflectComponent>()
		.expect("reflect type is not a component")
}

fn get_reflect_bundle(registry: &TypeRegistry, type_id: TypeId) -> &ReflectBundle {
	let type_registration = registry.get(type_id).expect("reflect type not registered");
	type_registration
		.data::<ReflectBundle>()
		.expect("reflect type is not a bundle")
}

#[cfg(test)]
mod tests {
	use crate::{AsyncEcsPlugin, AsyncWorld};
	use bevy::ecs::reflect::ReflectBundle;
	use bevy::prelude::*;
	use futures_lite::future;

	#[derive(Default, Component, Reflect)]
	#[reflect(Component)]
	struct Translation(u8, u8);

	#[derive(Default, Component, Reflect)]
	#[reflect(Component)]
	struct Scale(u8, u8);

	#[derive(Default, Bundle, Reflect)]
	#[reflect(Bundle)]
	struct Transform {
		translation: Translation,
		scale: Scale,
	}

	#[test]
	fn spawn() {
		let mut app = App::new();
		app.add_plugins((MinimalPlugins, AsyncEcsPlugin));

		app.register_type::<Translation>()
			.register_type::<Scale>()
			.register_type::<Transform>();

		let (sender, receiver) = async_channel::bounded(1);
		let async_world = AsyncWorld::from_world(&mut app.world);

		std::thread::spawn(move || {
			future::block_on(async move {
				let entity = async_world
					.spawn(Transform {
						translation: Translation(2, 3),
						scale: Scale(1, 1),
					})
					.await;
				sender.send(entity.id).await.unwrap();
			});
		});

		let id = loop {
			match receiver.try_recv() {
				Ok(id) => break id,
				Err(_) => app.update(),
			}
		};

		let translation = app.world.get::<Translation>(id).unwrap();
		assert_eq!(2, translation.0);
		assert_eq!(3, translation.1);
		let scale = app.world.get::<Scale>(id).unwrap();
		assert_eq!(1, scale.0);
		assert_eq!(1, scale.1);
	}

	#[test]
	fn insert_component() {
		let mut app = App::new();
		app.add_plugins((MinimalPlugins, AsyncEcsPlugin));

		app.register_type::<Translation>();

		let (sender, receiver) = async_channel::bounded(1);
		let async_world = AsyncWorld::from_world(&mut app.world);

		let thread = std::thread::spawn(move || {
			future::block_on(async move {
				let entity = async_world.spawn_empty().await;
				sender.send(entity.id).await.unwrap();

				entity.insert_component(Translation(2, 3)).await;
			});
		});

		let id = loop {
			match receiver.try_recv() {
				Ok(id) => break id,
				Err(_) => app.update(),
			}
		};

		while !thread.is_finished() {
			app.update();
		}
		app.update();

		let translation = app.world.get::<Translation>(id).unwrap();
		assert_eq!(2, translation.0);
		assert_eq!(3, translation.1);
	}

	#[test]
	fn insert_bundle() {
		let mut app = App::new();
		app.add_plugins((MinimalPlugins, AsyncEcsPlugin));

		app.register_type::<Translation>()
			.register_type::<Scale>()
			.register_type::<Transform>();

		let (sender, receiver) = async_channel::bounded(1);
		let async_world = AsyncWorld::from_world(&mut app.world);

		let thread = std::thread::spawn(move || {
			future::block_on(async move {
				let entity = async_world.spawn_empty().await;
				sender.send(entity.id).await.unwrap();

				entity
					.insert_bundle(Transform {
						translation: Translation(2, 3),
						scale: Scale(1, 1),
					})
					.await;
			});
		});

		let id = loop {
			match receiver.try_recv() {
				Ok(id) => break id,
				Err(_) => app.update(),
			}
		};

		while !thread.is_finished() {
			app.update();
		}
		app.update();

		let translation = app.world.get::<Translation>(id).unwrap();
		assert_eq!(2, translation.0);
		assert_eq!(3, translation.1);
		let scale = app.world.get::<Scale>(id).unwrap();
		assert_eq!(1, scale.0);
		assert_eq!(1, scale.1);
	}

	#[test]
	fn remove_component() {
		let mut app = App::new();
		app.add_plugins((MinimalPlugins, AsyncEcsPlugin));

		app.register_type::<Translation>();

		let async_world = AsyncWorld::from_world(&mut app.world);
		let id = app.world.spawn(Translation(3, 4)).id();

		let thread = std::thread::spawn(move || {
			future::block_on(async move {
				async_world
					.entity(id)
					.remove_component::<Translation>()
					.await;
			});
		});

		while !thread.is_finished() {
			app.update();
		}
		app.update();

		assert!(app.world.get::<Translation>(id).is_none());
	}

	#[test]
	fn remove_bundle() {
		let mut app = App::new();
		app.add_plugins((MinimalPlugins, AsyncEcsPlugin));

		app.register_type::<Translation>()
			.register_type::<Scale>()
			.register_type::<Transform>();

		let async_world = AsyncWorld::from_world(&mut app.world);
		let id = app
			.world
			.spawn(Transform {
				translation: Translation(3, 4),
				scale: Scale(1, 1),
			})
			.id();

		let thread = std::thread::spawn(move || {
			future::block_on(async move {
				async_world.entity(id).remove_bundle::<Transform>().await;
			});
		});

		while !thread.is_finished() {
			app.update();
		}
		app.update();

		assert!(app.world.get::<Translation>(id).is_none());
		assert!(app.world.get::<Scale>(id).is_none());
	}

	#[test]
	fn wait_for() {
		let mut app = App::new();
		app.add_plugins((MinimalPlugins, AsyncEcsPlugin));

		app.register_type::<Translation>()
			.register_type::<Scale>()
			.register_type::<Transform>();

		let (entity_tx, entity_rx) = async_channel::bounded(1);
		let (translation_tx, translation_rx) = async_channel::bounded(1);

		let async_world_1 = AsyncWorld::from_world(&mut app.world);
		let async_world_2 = async_world_1.clone();

		std::thread::spawn(move || {
			future::block_on(async move {
				let entity = async_world_1.spawn_empty().await;
				let component = entity.start_waiting_for::<Translation>().await;
				entity_tx.send(entity.id).await.unwrap();
				let translation = component.wait().await;
				translation_tx.send(translation).await.unwrap();
			});
		});

		std::thread::spawn(move || {
			future::block_on(async move {
				let id = entity_rx.recv().await.unwrap();
				let entity = async_world_2.entity(id);
				entity.insert_component(Translation(4, 5)).await;
			});
		});

		let translation = loop {
			match translation_rx.try_recv() {
				Ok(translation) => break translation,
				Err(_) => app.update(),
			}
		};

		assert_eq!(4, translation.0);
		assert_eq!(5, translation.1);
	}

	#[test]
	fn wait_for_immediate() {
		let mut app = App::new();
		app.add_plugins((MinimalPlugins, AsyncEcsPlugin));

		app.register_type::<Translation>()
			.register_type::<Scale>()
			.register_type::<Transform>();

		let id = app.world.spawn(Translation(1, 2)).id();
		let (translation_tx, translation_rx) = async_channel::bounded(1);
		let async_world = AsyncWorld::from_world(&mut app.world);

		std::thread::spawn(move || {
			future::block_on(async move {
				let entity = async_world.entity(id);
				let translation = entity.wait_for::<Translation>().await;
				translation_tx.send(translation).await.unwrap();
			});
		});

		let translation = loop {
			match translation_rx.try_recv() {
				Ok(translation) => break translation,
				Err(_) => app.update(),
			}
		};

		assert_eq!(1, translation.0);
		assert_eq!(2, translation.1);
	}

	#[test]
	fn insert_wait_remove() {
		let mut app = App::new();
		app.add_plugins((MinimalPlugins, AsyncEcsPlugin));

		app.register_type::<Translation>()
			.register_type::<Scale>()
			.register_type::<Transform>();

		let (value_tx, value_rx) = async_channel::bounded(1);
		let async_world = AsyncWorld::from_world(&mut app.world);
		let id = app.world.spawn_empty().id();

		std::thread::spawn(move || {
			future::block_on(async move {
				let scale: Scale = async_world.entity(id).insert_wait_remove(Scale(6, 7)).await;
				value_tx.send(scale).await.unwrap();
			});
		});

		let value = loop {
			match value_rx.try_recv() {
				Ok(value) => break value,
				Err(_) => app.update(),
			}
		};
		app.update();

		assert_eq!(6, value.0);
		assert_eq!(7, value.1);
		assert!(app.world.entity(id).get::<Scale>().is_none());
	}
}
