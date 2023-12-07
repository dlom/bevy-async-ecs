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
				info!("done! you can close the window");
			};
			AsyncComputeTaskPool::get().spawn(fut).detach();
		})
		.run();
}
