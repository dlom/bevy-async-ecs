use crate::util::remove_system;
use crate::world::AsyncWorld;
use crate::{die, recv_and_yield};
use bevy_ecs::prelude::*;
use bevy_ecs::system::{BoxedSystem, SystemId};
use std::any::Any;
use std::marker::PhantomData;
use std::sync::Arc;

type BoxedAnySend = Box<dyn Any + Send>;
type SystemIdWithIO = SystemId<BoxedAnySend, BoxedAnySend>;
type BoxedSystemWithIO = BoxedSystem<BoxedAnySend, BoxedAnySend>;

/// Represents a registered `System` that can be run asynchronously.
///
/// Dropping an `AsyncSystem` will not unregister it. Use `AsyncSystem::unregister()`
/// to clean up an `AsyncSystem` from the main bevy `World`.
///
/// The easiest way to get an `AsyncSystem` is with `AsyncWorld::register_system()`.
#[derive(Debug, Clone)]
pub struct AsyncSystem {
	id: Arc<SystemId>,
	world: AsyncWorld,
}

impl AsyncSystem {
	pub(crate) async fn new(system: BoxedSystem, world: AsyncWorld) -> Self {
		let (id_tx, id_rx) = async_channel::bounded(1);
		world
			.apply(move |world: &mut World| {
				let id = world.register_boxed_system(system);
				id_tx.try_send(id).unwrap_or_else(die);
			})
			.await;
		let id = recv_and_yield(id_rx).await;
		let id = Arc::new(id);
		Self { id, world }
	}

	/// Run the system.
	pub async fn run(&self) {
		let id = *self.id;
		self.world
			.apply(move |world: &mut World| {
				world.run_system(id).unwrap_or_else(die);
			})
			.await;
	}

	/// Unregister the system.
	///
	/// If multiple clones of the AsyncSystem exist, a reference counter will be
	/// decremented instead. The system will be unregistered when the counter
	/// decrements to zero.
	pub async fn unregister(self) {
		let Self { id, world } = self;
		if let Some(id) = Arc::into_inner(id) {
			world.apply(remove_system(id)).await;
		}
	}
}

/// Represents a registered `System` that accepts input and returns output, and can be run
/// asynchronously.
///
/// Dropping an `AsyncIOSystem` will not unregister it. Use `AsyncSystemIO::unregister()`
/// to clean up an `AsyncSystemIO` from the main bevy `World`.
///
/// The easiest way to get an `AsyncIOSystem` is with `AsyncWorld::register_io_system()`.
#[derive(Debug)]
pub struct AsyncIOSystem<I: Send, O: Send> {
	id: Arc<SystemIdWithIO>,
	world: AsyncWorld,
	_pd: PhantomData<fn(I) -> O>,
}

impl<I: Send, O: Send> Clone for AsyncIOSystem<I, O> {
	fn clone(&self) -> Self {
		Self {
			id: Arc::clone(&self.id),
			world: self.world.clone(),
			_pd: PhantomData,
		}
	}
}

impl<I: Send + 'static, O: Send + 'static> AsyncIOSystem<I, O> {
	pub(crate) async fn new<M>(system: impl IntoSystem<I, O, M> + Send, world: AsyncWorld) -> Self {
		fn unbox_input<I: Send + 'static>(In(boxed): In<BoxedAnySend>) -> I {
			let concrete = boxed.downcast().unwrap_or_else(die);
			*concrete
		}

		fn box_output<O: Send + 'static>(In(output): In<O>) -> BoxedAnySend {
			Box::new(output)
		}

		let system: BoxedSystemWithIO = Box::new(unbox_input.pipe(system).pipe(box_output));

		let (id_tx, id_rx) = async_channel::bounded(1);
		world
			.apply(move |world: &mut World| {
				let id = world.register_boxed_system(system);
				id_tx.try_send(id).unwrap_or_else(die);
			})
			.await;

		let id = recv_and_yield(id_rx).await;
		let id = Arc::new(id);

		Self {
			id,
			world,
			_pd: PhantomData,
		}
	}

	/// Run the system.
	pub async fn run(&self, input: I) -> O {
		let (input_tx, input_rx) = async_channel::bounded(1);
		let (output_tx, output_rx) = async_channel::bounded(1);

		let input: BoxedAnySend = Box::new(input);
		input_tx.send(input).await.unwrap_or_else(die);

		let id = *self.id;
		self.world
			.apply(move |world: &mut World| {
				let input = input_rx.try_recv().unwrap_or_else(die);
				let output = world.run_system_with_input(id, input).unwrap_or_else(die);
				output_tx.try_send(output).unwrap_or_else(die);
			})
			.await;

		let boxed: BoxedAnySend = recv_and_yield(output_rx).await;
		let concrete = boxed.downcast().unwrap_or_else(die);
		*concrete
	}

	/// Unregister the system.
	///
	/// If multiple clones of the AsyncIOSystem exist, a reference counter will be
	/// decremented instead. The system will be unregistered when the counter
	/// decrements to zero.
	pub async fn unregister(self) {
		let Self { id, world, _pd } = self;
		if let Some(id) = Arc::into_inner(id) {
			world.apply(remove_system(id)).await
		}
	}
}

#[cfg(test)]
mod tests {
	use crate::world::AsyncWorld;
	use crate::AsyncEcsPlugin;
	use bevy::ecs::system::RegisteredSystemError;
	use bevy::prelude::*;
	use bevy::tasks::AsyncComputeTaskPool;

	#[derive(Component)]
	struct Counter(u8);

	impl Counter {
		fn go_up(&mut self) {
			self.0 += 1;
		}
	}

	macro_rules! assert_counter {
		($id:expr, $value:expr, $world:expr) => {
			assert_eq!($value, $world.entity($id).get::<Counter>().unwrap().0);
		};
	}

	fn increase_counter_all(mut query: Query<&mut Counter>) {
		for mut counter in query.iter_mut() {
			counter.go_up();
		}
	}

	fn increase_counter(In(id): In<Entity>, mut query: Query<&mut Counter>) {
		let mut counter = query.get_mut(id).unwrap();
		counter.go_up();
	}

	fn get_counter_value(In(id): In<Entity>, query: Query<&Counter>) -> u8 {
		query.get(id).unwrap().0
	}

	#[test]
	fn smoke() {
		let mut app = App::new();
		app.add_plugins((MinimalPlugins, AsyncEcsPlugin));
		let id = app.world.spawn(Counter(0)).id();
		assert_counter!(id, 0, &app.world);

		let (barrier_tx, barrier_rx) = async_channel::bounded(1);
		let async_world = AsyncWorld::from_world(&mut app.world);

		AsyncComputeTaskPool::get()
			.spawn(async move {
				let increase_counter_all = async_world.register_system(increase_counter_all).await;
				increase_counter_all.run().await;
				barrier_tx.send(()).await.unwrap();
			})
			.detach();

		loop {
			match barrier_rx.try_recv() {
				Ok(_) => break,
				Err(_) => app.update(),
			}
		}
		app.update();

		assert_counter!(id, 1, &app.world);
	}

	#[test]
	fn normal_unregister() {
		let mut app = App::new();
		app.add_plugins((MinimalPlugins, AsyncEcsPlugin));
		let id = app.world.spawn(Counter(0)).id();
		assert_counter!(id, 0, &app.world);

		let (sender, receiver) = async_channel::bounded(1);
		let async_world = AsyncWorld::from_world(&mut app.world);

		AsyncComputeTaskPool::get()
			.spawn(async move {
				let increase_counter_all = async_world.register_system(increase_counter_all).await;
				let ica2 = increase_counter_all.clone();
				increase_counter_all.unregister().await;

				ica2.run().await;

				let id = *ica2.id;
				ica2.unregister().await;
				sender.send(id).await.unwrap();
			})
			.detach();

		let system_id = loop {
			match receiver.try_recv() {
				Ok(id) => break id,
				Err(_) => app.update(),
			}
		};
		app.update();

		let err = app.world.remove_system(system_id);
		assert_counter!(id, 1, &app.world);
		assert!(matches!(
			err,
			Err(RegisteredSystemError::SystemIdNotRegistered(_))
		));
	}

	#[test]
	fn io() {
		let mut app = App::new();
		app.add_plugins((MinimalPlugins, AsyncEcsPlugin));
		let id = app.world.spawn(Counter(4)).id();
		assert_counter!(id, 4, &app.world);

		let (sender, receiver) = async_channel::bounded(1);
		let async_world = AsyncWorld::from_world(&mut app.world);

		AsyncComputeTaskPool::get()
			.spawn(async move {
				let increase_counter = async_world
					.register_io_system::<Entity, (), _>(increase_counter)
					.await;
				let get_counter_value = async_world
					.register_io_system::<Entity, u8, _>(get_counter_value)
					.await;

				increase_counter.run(id).await;
				let value = get_counter_value.run(id).await;
				sender.send(value).await.unwrap();
			})
			.detach();

		let value = loop {
			match receiver.try_recv() {
				Ok(value) => break value,
				Err(_) => app.update(),
			}
		};
		app.update();

		assert_eq!(5, value);
		assert_counter!(id, 5, &app.world);
	}

	#[test]
	fn io_unregister() {
		let mut app = App::new();
		app.add_plugins((MinimalPlugins, AsyncEcsPlugin));
		let id = app.world.spawn(Counter(4)).id();
		assert_counter!(id, 4, &app.world);

		let (sender, receiver) = async_channel::bounded(1);
		let async_world = AsyncWorld::from_world(&mut app.world);

		AsyncComputeTaskPool::get()
			.spawn(async move {
				let increase_counter = async_world
					.register_io_system::<Entity, (), _>(increase_counter)
					.await;
				let get_counter_value = async_world
					.register_io_system::<Entity, u8, _>(get_counter_value)
					.await;

				let gcv2 = get_counter_value.clone();
				get_counter_value.unregister().await;

				increase_counter.run(id).await;
				let value = gcv2.run(id).await;
				sender.send((value, *gcv2.id)).await.unwrap();
				gcv2.unregister().await;
			})
			.detach();

		let (value, system_id) = loop {
			match receiver.try_recv() {
				Ok(value) => break value,
				Err(_) => app.update(),
			}
		};
		app.update();

		let err = app.world.remove_system(system_id);
		assert_eq!(5, value);
		assert_counter!(id, 5, &app.world);
		assert!(matches!(
			err,
			Err(RegisteredSystemError::SystemIdNotRegistered(_))
		));
	}
}
