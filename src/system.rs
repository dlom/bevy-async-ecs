use crate::{AsyncOperation, OperationSender};
use async_channel::{Receiver, Sender};
use bevy::ecs::system::{BoxedSystem, Command, SystemId};
use bevy::prelude::*;
use std::any::Any;
use std::marker::PhantomData;

pub(super) enum SystemOperation {
	Register(BoxedSystem, Sender<SystemId>),
	RegisterIO(AsyncIO, Sender<Entity>),
	MarkBeacon(Entity),
	UnmarkBeacon(Entity),
	Run(SystemId),
}

impl From<SystemOperation> for AsyncOperation {
	fn from(system_op: SystemOperation) -> Self {
		Self::System(system_op)
	}
}

impl Command for SystemOperation {
	fn apply(self, world: &mut World) {
		match self {
			SystemOperation::Register(system, sender) => {
				let id = world.register_boxed_system(system);
				sender.try_send(id).expect("invariant broken");
			}
			SystemOperation::RegisterIO(async_io, sender) => {
				let id = world.spawn(async_io).id();
				sender.try_send(id).expect("invariant broken");
			}
			SystemOperation::MarkBeacon(id) => {
				world.entity_mut(id).insert(AsyncIOBeacon);
			}
			SystemOperation::UnmarkBeacon(id) => {
				world.entity_mut(id).remove::<AsyncIOBeacon>();
			}
			SystemOperation::Run(id) => {
				world.run_system(id).expect("invariant broken");
			}
		}
	}
}

pub struct AsyncSystem {
	id: SystemId,
	sender: OperationSender,
}

impl AsyncSystem {
	pub(super) async fn new(system: BoxedSystem, sender: OperationSender) -> Self {
		let (id_sender, id_receiver) = async_channel::bounded(1);

		let operation = SystemOperation::Register(system, id_sender);
		sender.send(operation).await;

		let id = id_receiver.recv().await.expect("invariant broken");
		Self { id, sender }
	}

	pub async fn run(&self) {
		self.sender.send(SystemOperation::Run(self.id)).await;
	}
}

pub struct AsyncIOSystem<I: Send, O: Send> {
	beacon_location: Entity,
	sender: Sender<Box<dyn Any + Send>>,
	receiver: Receiver<Box<dyn Any + Send>>,
	inner: AsyncSystem,
	_pd: PhantomData<fn(I) -> O>,
}

impl<I: Send + 'static, O: Send + 'static> AsyncIOSystem<I, O> {
	pub(super) async fn new<M>(system: impl IntoSystem<I, O, M>, sender: OperationSender) -> Self {
		let (in_tx, in_rx) = async_channel::unbounded();
		let (out_tx, out_rx) = async_channel::unbounded();
		let async_io = AsyncIO(in_rx, out_tx);

		let (beacon_tx, beacon_rx) = async_channel::bounded(1);
		let operation = SystemOperation::RegisterIO(async_io, beacon_tx);
		sender.send(operation).await;
		let beacon_location = beacon_rx.recv().await.expect("invariant broken");

		fn receive_input<I: Send + 'static>(query: Query<&AsyncIO, With<AsyncIOBeacon>>) -> I {
			let AsyncIO(receiver, _) = query.get_single().expect("invariant broken");
			let receiver = receiver.clone();

			let boxed = receiver.try_recv().expect("invariant broken");
			let concrete = boxed.downcast().expect("invariant broken");
			*concrete
		}

		fn send_output<O: Send + 'static>(
			In(output): In<O>,
			query: Query<&AsyncIO, With<AsyncIOBeacon>>,
		) {
			let AsyncIO(_, sender) = query.get_single().expect("invariant broken");
			let sender = sender.clone();

			sender.try_send(Box::new(output)).expect("invariant broken");
		}

		let system = Box::new(receive_input.pipe(system).pipe(send_output));
		let inner = AsyncSystem::new(system, sender).await;

		Self {
			beacon_location,
			sender: in_tx,
			receiver: out_rx,
			inner,
			_pd: PhantomData,
		}
	}

	pub async fn run(&self, i: I) -> O {
		let i = Box::new(i);
		self.sender.send(i).await.expect("invariant broken");

		self.inner
			.sender
			.send(SystemOperation::MarkBeacon(self.beacon_location))
			.await;
		self.inner.run().await;
		self.inner
			.sender
			.send(SystemOperation::UnmarkBeacon(self.beacon_location))
			.await;

		let boxed = self.receiver.recv().await.expect("invariant broken");
		let concrete = boxed.downcast().expect("invariant broken");
		*concrete
	}
}

#[derive(Component)]
pub(super) struct AsyncIO(Receiver<Box<dyn Any + Send>>, Sender<Box<dyn Any + Send>>);

#[derive(Component)]
#[component(storage = "SparseSet")]
struct AsyncIOBeacon;

#[cfg(test)]
mod tests {
	use crate::{AsyncEcsPlugin, AsyncWorld};
	use bevy::prelude::*;
	use futures_lite::future;

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

	// Vanilla bevy systems

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

		std::thread::spawn(move || {
			future::block_on(async move {
				let increase_counter_all = async_world.register_system(increase_counter_all).await;
				increase_counter_all.run().await;
				barrier_tx.send(()).await.unwrap();
			});
		});

		loop {
			match barrier_rx.try_recv() {
				Ok(value) => break,
				Err(_) => app.update(),
			}
		}
		app.update();

		assert_counter!(id, 1, &app.world);
	}

	#[test]
	fn io() {
		let mut app = App::new();
		app.add_plugins((MinimalPlugins, AsyncEcsPlugin));
		let id = app.world.spawn(Counter(4)).id();
		assert_counter!(id, 4, &app.world);

		let (sender, receiver) = async_channel::bounded(1);
		let async_world = AsyncWorld::from_world(&mut app.world);

		let thread = std::thread::spawn(move || {
			future::block_on(async move {
				let increase_counter = async_world
					.register_io_system::<Entity, (), _>(increase_counter)
					.await;
				let get_counter_value = async_world
					.register_io_system::<Entity, u8, _>(get_counter_value)
					.await;

				increase_counter.run(id).await;
				let value = get_counter_value.run(id).await;
				sender.send(value).await.unwrap();
			});
		});

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
}
