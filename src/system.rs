use crate::{AsyncOperation, OperationSender};
use async_channel::Sender;
use bevy::ecs::prelude::World;
use bevy::ecs::system::{BoxedSystem, Command, SystemId};

pub enum SystemOperation {
	Register(BoxedSystem, Sender<SystemId>),
	Run(SystemId),
}

impl Command for SystemOperation {
	fn apply(self, world: &mut World) {
		match self {
			SystemOperation::Register(system, sender) => {
				let id = world.register_boxed_system(system);
				sender.try_send(id).expect("invariant broken");
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

		let operation = AsyncOperation::System(SystemOperation::Register(system, id_sender));
		sender.send(operation).await;

		let id = id_receiver.recv().await.expect("invariant broken");
		Self { id, sender }
	}

	pub async fn run(&self) {
		let operation = AsyncOperation::System(SystemOperation::Run(self.id));
		self.sender.send(operation).await;
	}
}

#[cfg(test)]
mod tests {
	use super::super::{AsyncEcsPlugin, AsyncWorld};
	use bevy::prelude::*;
	use futures_lite::future;

	#[derive(Component)]
	struct Counter(u8);

	macro_rules! assert_counter {
		($id:expr, $value:expr, $world:expr) => {
			assert_eq!($value, $world.entity($id).get::<Counter>().unwrap().0);
		};
	}

	fn increase_counter(mut query: Query<&mut Counter>) {
		for mut counter in query.iter_mut() {
			counter.0 += 1;
		}
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
				let increase_counter = async_world.register_system(increase_counter).await;
				barrier_tx.send(()).await.unwrap();
				increase_counter.run().await;
				barrier_tx.send(()).await.unwrap();
			});
		});

		app.update();
		barrier_rx.try_recv().unwrap();
		app.update();
		barrier_rx.try_recv().unwrap();

		assert_counter!(id, 1, &app.world);
	}
}
