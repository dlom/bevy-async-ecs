use crate::AsyncOperation;
use bevy::ecs::system::{Command, CommandQueue};
use bevy::prelude::*;
use std::fmt;

/// The object-safe equivalent of a `Box<dyn Command>`.
pub struct BoxedCommand(CommandQueue);

impl fmt::Debug for BoxedCommand {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.debug_struct("BoxedCommand").finish()
	}
}

impl BoxedCommand {
	/// Constructs a new `CommandBox` from the given Bevy command.
	pub fn new<C: Command>(inner: C) -> Self {
		Self({
			let mut queue = CommandQueue::default();
			queue.push(inner);
			queue
		})
	}
}

impl Command for BoxedCommand {
	fn apply(mut self, world: &mut World) {
		self.0.apply(world);
	}
}

impl From<BoxedCommand> for AsyncOperation {
	fn from(command: BoxedCommand) -> Self {
		Self::Command(command)
	}
}

#[cfg(test)]
mod tests {
	use crate::{AsyncEcsPlugin, AsyncWorld};
	use bevy::prelude::*;
	use futures_lite::future;

	use super::*;

	#[derive(Component)]
	struct Marker;

	#[test]
	fn smoke() {
		let mut app = App::new();
		app.add_plugins((MinimalPlugins, AsyncEcsPlugin));

		let async_world = AsyncWorld::from_world(&mut app.world);
		let (sender, receiver) = async_channel::bounded(1);
		let command = BoxedCommand::new(move |world: &mut World| {
			let id = world.spawn(Marker).id();
			sender.send_blocking(id).unwrap();
		});
		let debugged = format!("{:?}", command);

		std::thread::spawn(move || {
			future::block_on(async move {
				async_world.apply_command(command).await;
			});
		});

		let id = loop {
			match receiver.try_recv() {
				Ok(id) => break id,
				Err(_) => app.update(),
			}
		};
		app.update();

		assert!(app.world.entity(id).get::<Marker>().is_some());
		assert_eq!("BoxedCommand", debugged);
	}
}
