### Console sessions read votes

- **Fixed:** A console signed in with a browser session reads message votes and thread vote badges through `GET /ui/api/messages/{mid}/votes` and `GET /ui/api/threads/{tid}/votes`. Before, those paths were not routed, so vote tallies and card badges were empty for session users.
