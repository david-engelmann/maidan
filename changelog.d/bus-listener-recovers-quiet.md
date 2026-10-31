### A quiet server's event listener recovers its health

- **Fixed:** the Postgres `LISTEN` listener marked itself healthy again only when a notification arrived. After a failed reconnect (a hosted database briefly unreachable), a server with no traffic stayed `degraded`, and `/health` answered 503 indefinitely, which is what took dev.maidan.world's health down on 2026-10-27. While degraded, the listener now sends itself a probe NOTIFY every 5 seconds, so it reads healthy as soon as the path works again.
