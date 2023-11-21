use bevy::log::LogPlugin;
use bevy::prelude::*;
use bevy_async_ecs::*;
use futures_lite::future;

// vanilla Bevy system
fn print_names(query: Query<(Entity, &Name)>) {
	for (id, name) in query.iter() {
		info!("entity {:?} has name '{}'", id, name);
	}
}

fn main() {
	App::new()
		.add_plugins((MinimalPlugins, AsyncEcsPlugin, LogPlugin::default()))
		.add_systems(Startup, |world: &mut World| {
			let async_world = AsyncWorld::from_world(world);
			let fut = async move {
				let print_names = async_world.register_system(print_names).await;

				let entity = async_world.spawn_named("Frank").await;
				print_names.run().await;
				entity.despawn().await;
			};

			// In an non-example, you would use `AsyncComputeTaskPool::get().spawn(fut).detach()` instead
			std::thread::spawn(move || {
				future::block_on(fut);
			});
		})
		.run();
}
