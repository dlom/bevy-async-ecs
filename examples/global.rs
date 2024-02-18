use bevy::prelude::*;
use bevy::tasks::AsyncComputeTaskPool;
use bevy_async_ecs::*;
use rand::distributions::{Alphanumeric, Distribution};
use rand::rngs::SmallRng;
use rand::SeedableRng;
use std::sync::OnceLock;

// Setting up the global AsyncWorld

static ASYNC_WORLD: OnceLock<AsyncWorld> = OnceLock::new();

fn async_world() -> &'static AsyncWorld {
	ASYNC_WORLD
		.get()
		.expect("expected ASYNC_WORLD to be initialized")
}

fn init_async_world(world: &mut World) {
	ASYNC_WORLD
		.set(AsyncWorld::from_world(world))
		.expect("failed to initialize ASYNC_WORLD");
}

// Using the global AsyncWorld

#[derive(Event)]
struct EntitySpawned;

fn spawn_entity_via_async(input: Res<ButtonInput<KeyCode>>) {
	if input.just_pressed(KeyCode::Space) {
		AsyncComputeTaskPool::get()
			.spawn(async move {
				let mut rng = SmallRng::from_entropy();
				let name: String = Alphanumeric
					.sample_iter(&mut rng)
					.take(7)
					.map(char::from)
					.collect();

				let _entity = async_world().spawn_named(name).await;
				async_world().send_event(EntitySpawned).await;
			})
			.detach();
	}
}

// Vanilla Bevy system

fn print_names(query: Query<(Entity, &Name)>) {
	info!("enumerating all named entities:");
	for (id, name) in query.iter() {
		info!("  entity {:?} has name '{}'", id, name);
	}
}

fn main() {
	App::new()
		.add_event::<EntitySpawned>()
		.add_plugins((DefaultPlugins, AsyncEcsPlugin))
		.add_systems(Startup, init_async_world)
		.add_systems(Update, spawn_entity_via_async)
		.add_systems(Update, print_names.run_if(on_event::<EntitySpawned>()))
		.run();
}
