### Review child routing is published live

- **Fixed:** A change entering review now publishes its spawned review child's `thread_created` event and a new `thread_attributes_set` event carrying the child's kind, review family and author family, so live subscribers see the routing (#1572).
