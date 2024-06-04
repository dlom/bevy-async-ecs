# 🔄 Bevy Async ECS

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](https://opensource.org/licenses/MIT)
[![Doc](https://docs.rs/bevy-async-ecs/badge.svg)](https://docs.rs/bevy-async-ecs)
[![Crate](https://img.shields.io/crates/v/bevy-async-ecs.svg)](https://crates.io/crates/bevy-async-ecs)
[![Bevy tracking](https://img.shields.io/badge/Bevy%20tracking-released%20version-lightblue)](https://github.com/bevyengine/bevy/blob/main/docs/plugins_guidelines.md#main-branch-tracking)

## What is Bevy Async ECS?

Bevy Async ECS is an asynchronous interface to the standard Bevy `World`.
It aims to be simple and intuitive to use for those familiar with Bevy's ECS.

## `AsyncWorld`

`AsyncWorld` is the entrypoint for all further asynchronous manipulation of the world.
It can only be created using the `FromWorld` trait implementation.
It should be driven by an executor running parallel with the main Bevy app
(this can either be one of the `TaskPool`s or a blocking executor running on another thread).

Internally, the `AsyncWorld` simply wraps an MPSC channel sender.
As such, it can be cheaply cloned and further sent to separate threads or tasks.
This means that all operations on the `AsyncWorld` are processed in FIFO order.
However, there are no ordering guarantees between `AsyncWorld`s or any derivatives sharing the same internal channel
sender, or any `AsyncWorld`s constructed separately. 

It is important to note that Bevy is still running and mutating the world while the async tasks run! Assume that the
world could have been mutated between any asynchronous call. However, there are several ways to ensure that multiple commands
are applied together, without mutation of the world in between:
* Construct a vanilla Bevy `CommandQueue`, and send it to the Bevy `World` with `CommandQueueSender::send_queue()`
* Use the queue builder provided by the `AsyncWorld` via `AsyncWorld::start_queue()`

## Basic example

```rust
use bevy::prelude::*;
use bevy::tasks::AsyncComputeTaskPool;
use bevy_async_ecs::*;

// vanilla Bevy system
fn print_names(query: Query<(Entity, &Name)>) {
	for (id, name) in query.iter() {
		info!("entity {:?} has name '{}'", id, name);
	}
}

fn main() {
	App::new()
		.add_plugins((DefaultPlugins, AsyncEcsPlugin))
		.add_systems(Startup, |world: &mut World| {
			let async_world = AsyncWorld::from_world(world);
			let fut = async move {
				let print_names = async_world.register_system(print_names).await;

				let entity = async_world.spawn_named("Frank").await;
				print_names.run().await;
				entity.despawn().await;
			};
			AsyncComputeTaskPool::get().spawn(fut).detach();
		})
		.run();
}
```

## Multithreaded

`bevy-async-ecs` does not explicitly require the `multi-threaded` feature (though all the tests and examples do).
However, when the task executor is running on a single thread (on wasm, for example), the async world will probably
deadlock. If this is a pain-point for you, please open a GitHub issue.

## Most recently compatible versions

| bevy | bevy-async-ecs |
|------|----------------|
| 0.13 | 0.5.1          |
| 0.12 | 0.4.1          |
| 0.11 | N/A            |
