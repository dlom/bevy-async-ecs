use crate::AsyncOperation;
use async_channel::{Receiver, Sender};
use bevy::ecs::system::Command;
use bevy::prelude::*;
use bevy::reflect::TypeRegistry;
use std::any::TypeId;
use std::marker::PhantomData;

pub(crate) enum ResourceOperation {
	Insert(Box<dyn Reflect>),
	Remove(TypeId),
	WaitFor(TypeId, Sender<Box<dyn Reflect>>),
}

impl Command for ResourceOperation {
	fn apply(self, world: &mut World) {
		world.resource_scope(|world, registry: Mut<AppTypeRegistry>| {
			let registry = registry.read();
			match self {
				ResourceOperation::Insert(boxed) => {
					let reflect_resource = get_reflect_resource(&registry, (*boxed).type_id());
					reflect_resource.apply_or_insert(world, boxed.as_reflect());
				}
				ResourceOperation::Remove(type_id) => {
					let reflect_resource = get_reflect_resource(&registry, type_id);
					reflect_resource.remove(world);
				}
				ResourceOperation::WaitFor(type_id, sender) => {
					let reflect_resource = get_reflect_resource(&registry, type_id);
					if let Some(reflect) = reflect_resource.reflect(world) {
						sender
							.try_send(reflect.clone_value())
							.expect("invariant broken");
					} else {
						world.spawn(WaitingFor(type_id, sender));
					}
				}
			}
		});
	}
}

impl From<ResourceOperation> for AsyncOperation {
	fn from(resource_op: ResourceOperation) -> Self {
		Self::Resource(resource_op)
	}
}

fn get_reflect_resource(registry: &TypeRegistry, type_id: TypeId) -> &ReflectResource {
	let type_registration = registry.get(type_id).expect("reflect type not registered");
	type_registration
		.data::<ReflectResource>()
		.expect("reflect type is not a resource")
}

#[derive(Component)]
pub(crate) struct WaitingFor(TypeId, Sender<Box<dyn Reflect>>);

pub(crate) fn wait_for_reflect_resources(
	mut commands: Commands,
	query: Query<(Entity, &WaitingFor)>,
	registry: Res<AppTypeRegistry>,
	world: &World,
) {
	let registry = registry.read();
	for (id, WaitingFor(type_id, sender)) in query.iter() {
		let reflect_resource = get_reflect_resource(&registry, *type_id);
		if let Some(reflect) = reflect_resource.reflect(world) {
			sender
				.try_send(reflect.clone_value())
				.expect("invariant broken");
			commands.entity(id).despawn();
		}
	}
}

/// Represents a `Resource` being retrieved.
pub struct AsyncResource<R>(Receiver<Box<dyn Reflect>>, PhantomData<R>);

impl<R: Resource + FromReflect> AsyncResource<R> {
	pub(crate) fn new(receiver: Receiver<Box<dyn Reflect>>) -> Self {
		Self(receiver, PhantomData)
	}

	/// Wait for the `Resource` to exist, and retrieve its value.
	pub async fn wait(self) -> R {
		let boxed_dynamic = self.0.recv().await.expect("invariant broken");
		R::take_from_reflect(boxed_dynamic).expect("invariant broken")
	}
}

#[cfg(test)]
mod tests {
	use crate::world::AsyncWorld;
	use crate::AsyncEcsPlugin;
	use bevy::prelude::*;
	use futures_lite::future;

	#[derive(Default, Resource, Reflect)]
	#[reflect(Resource)]
	struct Counter(u8);

	#[test]
	fn insert() {
		let mut app = App::new();
		app.add_plugins((MinimalPlugins, AsyncEcsPlugin));
		app.register_type::<Counter>();

		let async_world = AsyncWorld::from_world(&mut app.world);
		let (sender, receiver) = async_channel::bounded(1);

		std::thread::spawn(move || {
			future::block_on(async move {
				async_world.insert_resource(Counter(4)).await;
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

		assert_eq!(4, app.world.resource::<Counter>().0);
	}

	#[test]
	fn remove() {
		let mut app = App::new();
		app.add_plugins((MinimalPlugins, AsyncEcsPlugin));
		app.register_type::<Counter>();

		let async_world = AsyncWorld::from_world(&mut app.world);
		let (sender, receiver) = async_channel::bounded(1);
		app.insert_resource(Counter(7));

		std::thread::spawn(move || {
			future::block_on(async move {
				async_world.remove_resource::<Counter>().await;
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

		assert!(app.world.get_resource::<Counter>().is_none());
	}

	#[test]
	fn wait_for() {
		let mut app = App::new();
		app.add_plugins((MinimalPlugins, AsyncEcsPlugin));
		app.register_type::<Counter>();

		let async_world_1 = AsyncWorld::from_world(&mut app.world);
		let async_world_2 = async_world_1.clone();
		let (barrier_tx, barrier_rx) = async_channel::bounded(1);
		let (value_tx, value_rx) = async_channel::bounded(1);

		std::thread::spawn(move || {
			future::block_on(async move {
				let resource = async_world_1.start_waiting_for_resource::<Counter>().await;
				barrier_tx.send(()).await.unwrap();
				let counter = resource.wait().await;
				value_tx.send(counter.0).await.unwrap();
			});
		});

		std::thread::spawn(move || {
			future::block_on(async move {
				barrier_rx.recv().await.unwrap();
				async_world_2.insert_resource(Counter(3)).await;
			});
		});

		let value = loop {
			match value_rx.try_recv() {
				Ok(value) => break value,
				Err(_) => app.update(),
			}
		};
		app.update();

		assert_eq!(3, value);
	}

	#[test]
	fn wait_for_immediate() {
		let mut app = App::new();
		app.add_plugins((MinimalPlugins, AsyncEcsPlugin));
		app.register_type::<Counter>();

		app.insert_resource(Counter(1));

		let async_world = AsyncWorld::from_world(&mut app.world);
		let (value_tx, value_rx) = async_channel::bounded(1);

		std::thread::spawn(move || {
			future::block_on(async move {
				let counter = async_world.wait_for_resource::<Counter>().await;
				value_tx.send(counter.0).await.unwrap();
			});
		});

		let value = loop {
			match value_rx.try_recv() {
				Ok(value) => break value,
				Err(_) => app.update(),
			}
		};
		app.update();

		assert_eq!(1, value);
	}
}
