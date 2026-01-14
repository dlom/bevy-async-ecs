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

use crate::command::receive_and_apply_commands;
use crate::wait_for::drive_waiting_for;
use crate::wait_for::initialize_waiters;
use async_channel::Receiver;
use bevy_app::prelude::*;
use bevy_ecs::prelude::*;
use futures_lite::Stream;
use pin_project_lite::pin_project;
use std::pin::Pin;
use std::task::Context;
use std::task::Poll;
use std::task::ready;

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

pin_project! {
	struct WorldFuture<T> {
		#[pin]
		rx: Receiver<T>,
	}
}

impl<T> Future for WorldFuture<T> {
	type Output = T;

	fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
		let rx = self.project().rx;
		let output = ready!(rx.poll_next(cx));
		output.map(Poll::Ready).unwrap_or_else(|| {
			bevy_log::debug!("oneshot rx was terminated, is the app closing?");
			bevy_log::debug!("(this task will now pend until the app closes)");
			Poll::Pending
		})
	}
}

async fn recv<T: Send>(rx: Receiver<T>) -> T {
	WorldFuture { rx }.await
}

/// Adds asynchronous ECS operations to Bevy `App`s.
#[derive(Debug)]
pub struct AsyncEcsPlugin;

impl Plugin for AsyncEcsPlugin {
	fn build(&self, app: &mut App) {
		app.add_systems(PreStartup, initialize_waiters)
			.add_systems(Last, (receive_and_apply_commands, ApplyDeferred).chain())
			.add_systems(PostUpdate, (drive_waiting_for, ApplyDeferred).chain());
	}
}

#[cfg(test)]
mod tests {
	use crate::recv;
	use pollster::block_on;

	#[test]
	fn no_die() {
		let (tx, rx) = async_channel::bounded::<u8>(1);
		tx.try_send(3).unwrap();
		assert_eq!(3, block_on(recv(rx)));
	}
}
