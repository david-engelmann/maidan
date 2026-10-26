### Renaming yourself is announced and audited

- **Fixed:** Changing a display name with `PATCH /members/{id}` updated the member silently, so live surfaces kept showing the old name and the audit log had no record of it. The change now appends a `member_display_name_changed` event and a `member.display_name.set` audit row in the same transaction, on both backends, and publishes the event to the member's own workspace only. The event is not federated.
