use crate::operations::OperationQueue;
use crate::{AsyncOperation, OperationSender};
use async_channel::{Receiver, Sender};
use bevy_ecs::prelude::*;
use bevy_ecs::system::{BoxedSystem, Command, SystemId};
use std::any::Any;
use std::marker::PhantomData;

// TODO(Bevy 0.13):
// The AsyncIO and AsyncIOBeacon structs are hacks to enable sending IO to/from a system
// until Bevy 0.13 lands with https://github.com/bevyengine/bevy/pull/10380.
// When that happens, this file should be reverted to how it was in 886204a201eb94e54d85c8fb88d5cc722042d244.

type AnyReceiver = Receiver<Box<dyn Any + Send>>;
type AnySender = Sender<Box<dyn Any + Send>>;

/// A `System`-related operation that can be applied to an `AsyncWorld`.
#[derive(Debug)]
#[non_exhaustive]
pub enum SystemOperation {
	/// Register the `System` with the `AsyncWorld`. The registered system's ID will be sent into the `Sender`.
	Register(BoxedSystem, Sender<SystemId>),
	/// Spawn an entity with the IO channels attached. The spawned entity's ID will be sent into the `Sender`.
	RegisterIO(AsyncIO, Sender<Entity>),
	/// Add the `AsyncIOBeacon` `Component` to the given `Entity`.
	MarkBeacon(Entity),
	/// Remove the `AsyncIOBeacon` `Component` from the given `Entity`.
	UnmarkBeacon(Entity),
	/// Run the system specified by the `SystemId` on the given `AsyncWorld`.
	Run(SystemId),
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

impl From<SystemOperation> for AsyncOperation {
	fn from(system_op: SystemOperation) -> Self {
		Self::System(system_op)
	}
}

/// A `Component` for vanilla Bevy that facilitates receiving and sending values to an async context.
#[derive(Debug, Component)]
#[component(storage = "SparseSet")]
pub struct AsyncIO {
	input_rx: AnyReceiver,
	output_tx: AnySender,
}

impl AsyncIO {
	/// Construct a new `AsyncIO` from an input `Receiver` and an output `Sender`.
	pub fn new(input_rx: AnyReceiver, output_tx: AnySender) -> Self {
		Self {
			input_rx,
			output_tx,
		}
	}

	/// Synchronously receive input from the async context.
	pub fn receive_input(&self) -> Box<dyn Any + Send> {
		self.input_rx.try_recv().expect("invariant broken")
	}

	/// Synchronously send output back to the async context.
	pub fn send_output(&self, value: Box<dyn Any + Send>) {
		self.output_tx.try_send(value).expect("invariant broken")
	}
}

/// The marker `Component` that is manipulated by `SystemOperation::MarkBeacon` and `SystemOperation::UnmarkBeacon`.
#[derive(Component)]
#[component(storage = "SparseSet")]
pub struct AsyncIOBeacon;

/// Represents a registered `System` that can be run asynchronously.
#[derive(Debug)]
pub struct AsyncSystem {
	id: SystemId,
	sender: OperationSender,
}

impl AsyncSystem {
	pub(crate) async fn new(system: BoxedSystem, sender: OperationSender) -> Self {
		let (id_sender, id_receiver) = async_channel::bounded(1);

		let operation = SystemOperation::Register(system, id_sender);
		sender.send(operation).await;

		let id = id_receiver.recv().await.expect("invariant broken");
		Self { id, sender }
	}

	/// Run the system.
	pub async fn run(&self) {
		self.sender.send(SystemOperation::Run(self.id)).await;
	}
}

/// Represents a registered `System` that accepts input and returns output, and can be run
/// asynchronously.
#[derive(Debug)]
pub struct AsyncIOSystem<I: Send, O: Send> {
	beacon_location: Entity,
	input_tx: AnySender,
	output_rx: AnyReceiver,
	inner: AsyncSystem,
	_pd: PhantomData<fn(I) -> O>,
}

impl<I: Send + 'static, O: Send + 'static> AsyncIOSystem<I, O> {
	pub(crate) async fn new<M>(system: impl IntoSystem<I, O, M>, sender: OperationSender) -> Self {
		let (input_tx, input_rx) = async_channel::unbounded();
		let (output_tx, output_rx) = async_channel::unbounded();
		let (beacon_tx, beacon_rx) = async_channel::bounded(1);

		let async_io = AsyncIO::new(input_rx, output_tx);
		let operation = SystemOperation::RegisterIO(async_io, beacon_tx);
		sender.send(operation).await;
		let beacon_location = beacon_rx.recv().await.expect("invariant broken");

		fn receive_input<I: Send + 'static>(query: Query<&AsyncIO, With<AsyncIOBeacon>>) -> I {
			let async_io = query.get_single().expect("invariant broken");
			let boxed = async_io.receive_input();
			let concrete = boxed.downcast().expect("invariant broken");
			*concrete
		}

		fn send_output<O: Send + 'static>(
			In(output): In<O>,
			query: Query<&AsyncIO, With<AsyncIOBeacon>>,
		) {
			let async_io = query.get_single().expect("invariant broken");
			async_io.send_output(Box::new(output));
		}

		let system = Box::new(receive_input.pipe(system).pipe(send_output));
		let inner = AsyncSystem::new(system, sender).await;

		Self {
			beacon_location,
			input_tx,
			output_rx,
			inner,
			_pd: PhantomData,
		}
	}

	/// Run the system.
	pub async fn run(&self, i: I) -> O {
		let i = Box::new(i);
		self.input_tx.send(i).await.expect("invariant broken");

		let operation = {
			let mut queue = OperationQueue::default();
			queue.push(SystemOperation::MarkBeacon(self.beacon_location));
			queue.push(SystemOperation::Run(self.inner.id));
			queue.push(SystemOperation::UnmarkBeacon(self.beacon_location));
			queue
		};
		self.inner.sender.send(operation).await;

		let boxed = self.output_rx.recv().await.expect("invariant broken");
		let concrete = boxed.downcast().expect("invariant broken");
		*concrete
	}
}

#[cfg(test)]
mod tests {
	use crate::world::AsyncWorld;
	use crate::AsyncEcsPlugin;
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
				Ok(_) => break,
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

		std::thread::spawn(move || {
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
