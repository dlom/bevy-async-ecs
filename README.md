# Bevy Async ECS

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
However, there are no ordering guarantees between `AsyncWorld`s or any derivatives sharing the same internal channel sender.
If ordering guarantees are required, you can use the operations API described below (`AsyncOperation::Queue`).

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

## Operations

The internal API used to implement the `AsyncWorld` is exposed and documented through the `operations` module. 
`AsyncOperation`s can be applied directly to an `AsyncWorld`. 
