#![forbid(unsafe_code)]
#![warn(missing_debug_implementations)]
#![warn(missing_docs)]
#![doc = include_str!("../README.md")]

mod command;
mod entity;
mod resource;
mod system;
mod world;

use bevy_app::prelude::*;
use bevy_ecs::prelude::*;
use entity::wait_for_reflect_components;
use operation::AsyncOperation;
use resource::wait_for_reflect_resources;
use std::borrow::Cow;

use crate::operation::{apply_operations, receive_operations, WorldOperationQueue};
pub use entity::{AsyncComponent, AsyncEntity};
pub use resource::AsyncResource;
pub use system::{AsyncIOSystem, AsyncSystem};
pub use world::AsyncWorld;

/// Types for interacting with the `AsyncWorld` directly, rather than through the convenience commands.
pub mod operation;

type CowStr = Cow<'static, str>;

/// Adds asynchronous ECS operations to Bevy `App`s.
#[derive(Debug)]
pub struct AsyncEcsPlugin;

impl Plugin for AsyncEcsPlugin {
	fn build(&self, app: &mut App) {
		app.init_resource::<WorldOperationQueue>()
			.add_systems(
				Last,
				(receive_operations, apply_operations, apply_deferred).chain(),
			)
			.add_systems(
				PostUpdate,
				(wait_for_reflect_components, wait_for_reflect_resources),
			);
	}
}
