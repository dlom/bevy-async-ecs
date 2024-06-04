# 0.5.1

- Added `AsyncSystem::unregister()` and `AsyncIOSystem::unregister()`

# 0.5.0

- Updated for bevy 0.13

# 0.4.1

- `impl Clone` for `AsyncSystem` and `AsyncIOSystem`

# 0.4.0

- Properly exposed `CommandQueueSender` as a public API.
- Doc improvements

# 0.3.0

- (Mostly) complete rewrite of internal API.
- Removed dependence on `bevy_reflect`.
- `operation` module has been removed.
- `OperationSender` is now `CommandQueueSender`.
- Public API is... mostly unchanged!
- Unified component operations (`insert_component` and `insert_bundle` became `insert`, etc...).
- Event support (`send_event`, `wait_for_event`, etc...).

# 0.2.0

- `OperationSender` is now fully exposed as an internal API.

# 0.1.0

- Initial release.
