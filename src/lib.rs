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

use crate::command::{apply_commands, initialize_command_queue, receive_commands};
use crate::wait_for::{drive_waiting_for, initialize_waiters};
use async_channel::Receiver;
use bevy_app::prelude::*;
use bevy_ecs::prelude::*;

pub use command::{BoxedCommand, CommandQueueBuilder, CommandQueueSender};
pub use entity::{AsyncComponent, AsyncEntity};
pub use system::{AsyncIOSystem, AsyncSystem};
pub use world::{AsyncEvents, AsyncResource, AsyncWorld};

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
		app.add_systems(PreStartup, (initialize_command_queue, initialize_waiters))
			.add_systems(
				Last,
				(receive_commands, apply_commands, apply_deferred).chain(),
			)
			.add_systems(PostUpdate, (drive_waiting_for, apply_deferred).chain());
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
