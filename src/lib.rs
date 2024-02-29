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
use futures_lite::{future, pin};

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

async fn recv_and_yield<T: Send>(receiver: Receiver<T>) -> T {
	let recv_fut = receiver.recv();
	pin!(recv_fut);
	loop {
		if let Some(value) = future::poll_once(recv_fut.as_mut()).await {
			return value.unwrap_or_else(die);
		} else {
			future::yield_now().await;
		}
	}
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
	use crate::recv_and_yield;
	use futures_lite::{future, pin};

	#[test]
	#[should_panic(expected = "invariant broken: RecvError")]
	fn die() {
		let (rx, tx) = async_channel::bounded::<()>(1);
		rx.close();
		future::block_on(recv_and_yield(tx));
	}

	#[test]
	fn no_die() {
		let (tx, rx) = async_channel::bounded::<u8>(1);
		let fut = recv_and_yield(rx);
		pin!(fut);
		assert!(future::block_on(future::poll_once(&mut fut)).is_none());
		assert!(future::block_on(future::poll_once(&mut fut)).is_none());
		assert!(future::block_on(future::poll_once(&mut fut)).is_none());
		tx.try_send(3).unwrap();
		assert_eq!(3, future::block_on(fut));
	}
}
