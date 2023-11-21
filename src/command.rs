use bevy::ecs::system::{Command, CommandQueue};
use bevy::prelude::*;

pub struct BoxedCommand(CommandQueue);

impl BoxedCommand {
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
