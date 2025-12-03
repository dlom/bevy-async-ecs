#![forbid(unsafe_code)]
#![warn(missing_debug_implementations)]
#![warn(missing_docs)]
#![warn(clippy::future_not_send)]
#![doc = include_str!("../README.md")]

mod command;
mod entity;
mod system;
mod util;
mod wait_for;
mod world;

use crate::wait_for::drive_waiting_for;
use crate::wait_for::initialize_waiters;
use async_channel::Receiver;
use bevy_app::prelude::*;
use bevy_ecs::prelude::*;
use bevy_malek_async::AsyncEcsPlugin as MalekPlugin;

pub use command::BoxedCommand;
pub use command::CommandQueueBuilder;
pub use command::CommandQueueSender;
pub use entity::AsyncComponent;
pub use entity::AsyncEntity;
pub use system::AsyncIOSystem;
pub use system::AsyncSystem;
pub use world::AsyncMessages;
pub use world::AsyncResource;
pub use world::AsyncWorld;

type CowStr = std::borrow::Cow<'static, str>;

#[inline(never)]
#[cold]
#[track_caller]
fn die<T, E: std::fmt::Debug>(e: E) -> T {
	panic!("invariant broken: {:?}", e)
}

async fn recv<T: Send>(receiver: Receiver<T>) -> T {
	receiver.recv().await.unwrap_or_else(die)
}

/// Adds asynchronous ECS operations to Bevy `App`s.
#[derive(Debug)]
pub struct AsyncEcsPlugin;

impl Plugin for AsyncEcsPlugin {
	fn build(&self, app: &mut App) {
		if app.get_added_plugins::<MalekPlugin>().is_empty() {
			app.add_plugins(MalekPlugin);
		}

		app.add_systems(PreStartup, initialize_waiters)
			.add_systems(Last, || {
				// nop
			})
			.add_systems(PostUpdate, (drive_waiting_for, ApplyDeferred).chain());
	}
}

#[cfg(test)]
mod tests {
	use crate::recv;
	use pollster::block_on;

	#[test]
	#[should_panic(expected = "invariant broken: RecvError")]
	fn die() {
		let (tx, rx) = async_channel::bounded::<()>(1);
		tx.close();
		block_on(recv(rx));
	}

	#[test]
	fn no_die() {
		let (tx, rx) = async_channel::bounded::<u8>(1);
		tx.try_send(3).unwrap();
		assert_eq!(3, block_on(recv(rx)));
	}
}
