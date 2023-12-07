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
