### Imported thread holders count as having worked the thread

- **Fixed:** Importing a workspace kept each thread's holder but did not record that they had worked the thread, so a holder who later released it passed the past-holder checks as someone who never held it. Import now records the holder on both backends, as a claim does.
