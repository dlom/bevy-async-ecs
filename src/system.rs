use crate::{AsyncOperation, OperationSender};
use async_channel::{Receiver, Sender};
use bevy::ecs::system::{BoxedSystem, Command, SystemId};
use bevy::prelude::*;
use std::any::Any;
use std::marker::PhantomData;

type BoxedAnySend = Box<dyn Any + Send>;
type SystemIdWithIO = SystemId<BoxedAnySend, BoxedAnySend>;
type BoxedSystemWithIO = BoxedSystem<BoxedAnySend, BoxedAnySend>;

/// A `System`-related operation that can be applied to an `AsyncWorld`.
#[derive(Debug)]
#[non_exhaustive]
pub enum SystemOperation {
	/// Register the `System` with the `AsyncWorld`. The registered system's ID will be sent into the `Sender`.
	Register(BoxedSystem, Sender<SystemId>),
	/// Register the `System` with the `AsyncWorld`. The registered system's ID will be sent into the `Sender`.
	RegisterWithIO(BoxedSystemWithIO, Sender<SystemIdWithIO>),
	/// Run the system specified by the `SystemId` on the given `AsyncWorld`.
	Run(SystemId),
	/// Run the system specified by the `SystemId` on the given `AsyncWorld`. Input and output will be
	/// received and sent on the respective channels.
	RunWithIO(SystemIdWithIO, Receiver<BoxedAnySend>, Sender<BoxedAnySend>),
}

impl Command for SystemOperation {
	fn apply(self, world: &mut World) {
		match self {
			SystemOperation::Register(system, id_tx) => {
				let id = world.register_boxed_system(system);
				id_tx.try_send(id).expect("invariant broken");
			}
			SystemOperation::RegisterWithIO(system, id_tx) => {
				let id = world.register_boxed_system(system);
				id_tx.try_send(id).expect("invariant broken");
			}
			SystemOperation::Run(id) => {
				world.run_system(id).expect("invariant broken");
			}
			SystemOperation::RunWithIO(id, input_rx, output_tx) => {
				let input = input_rx.try_recv().expect("invariant broken");
				let output = world
					.run_system_with_input(id, input)
					.expect("invariant broken");
				output_tx.try_send(output).expect("invariant broken");
			}
		}
	}
}

impl From<SystemOperation> for AsyncOperation {
	fn from(system_op: SystemOperation) -> Self {
		Self::System(system_op)
	}
}

/// Represents a registered `System` that can be run asynchronously.
#[derive(Debug)]
pub struct AsyncSystem {
	id: SystemId,
	operation_tx: OperationSender,
}

impl AsyncSystem {
	pub(crate) async fn new(system: BoxedSystem, operation_tx: OperationSender) -> Self {
		let (it_tx, id_rx) = async_channel::bounded(1);
		let operation = SystemOperation::Register(system, it_tx);
		operation_tx.send(operation).await;
		let id = id_rx.recv().await.expect("invariant broken");

		Self { id, operation_tx }
	}

	/// Run the system.
	pub async fn run(&self) {
		self.operation_tx.send(SystemOperation::Run(self.id)).await;
	}
}

/// Represents a registered `System` that accepts input and returns output, and can be run
/// asynchronously.
#[derive(Debug)]
pub struct AsyncIOSystem<I: Send, O: Send> {
	id: SystemIdWithIO,
	operation_tx: OperationSender,
	_pd: PhantomData<fn(I) -> O>,
}

impl<I: Send + 'static, O: Send + 'static> AsyncIOSystem<I, O> {
	pub(crate) async fn new<M>(
		system: impl IntoSystem<I, O, M>,
		operation_tx: OperationSender,
	) -> Self {
		fn unbox_input<I: Send + 'static>(In(boxed): In<BoxedAnySend>) -> I {
			let concrete = boxed.downcast().expect("invariant broken");
			*concrete
		}

		fn box_output<O: Send + 'static>(In(output): In<O>) -> BoxedAnySend {
			Box::new(output)
		}

		let system = Box::new(unbox_input.pipe(system).pipe(box_output));

		let (id_tx, id_rx) = async_channel::bounded(1);
		let operation = SystemOperation::RegisterWithIO(system, id_tx);
		operation_tx.send(operation).await;
		let id = id_rx.recv().await.expect("invariant broken");

		Self {
			id,
			operation_tx,
			_pd: PhantomData,
		}
	}

	/// Run the system.
	pub async fn run(&self, input: I) -> O {
		let (input_tx, input_rx) = async_channel::bounded(1);
		let (output_tx, output_rx) = async_channel::bounded(1);

		let input: BoxedAnySend = Box::new(input);
		input_tx.send(input).await.expect("invariant broken");

		let operation = SystemOperation::RunWithIO(self.id, input_rx, output_tx);
		self.operation_tx.send(operation).await;

		let boxed = output_rx.recv().await.expect("invariant broken");
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
