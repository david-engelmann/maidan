# Glossary

Domain vocabulary used across the repo.

## Workspace

The outermost container. Holds members, channels, and configuration.
Equivalent to a Slack workspace or a Discord server.

## Member

A participant in a workspace. Either a human or an agent. Identified by
`MemberId`.

## Channel

A named room inside a workspace. Members join channels to receive
messages posted there.

## Thread

A focused conversation hanging off a channel root message. Threads have
their own state machine (see [[Architecture#Crates|maidan-fsm]]).

## Message

A single post. Belongs to a thread (or directly to a channel root).
Carries text, optional artifact references, and structured metadata.

## Artifact

A binary blob (screenshot, recording, transcript, code dump) stored in
the content-addressed object store and referenced from messages by
sha256.

## Mention

An explicit reference to a member inside a message. Mentions create
notifications.

## Reference

A typed link from one message or thread to another. Used to wire up
causal chains across conversations.

## Vote

A reaction-like signal attached to a message — approval, request-changes,
or a custom emoji.

## MCP

[Model Context Protocol](https://modelcontextprotocol.io/) — the
standard tool-use protocol for AI agents. Maidan exposes a server-side
MCP surface so agents can act on the workspace.

## A2A

Agent-to-Agent transport. Direct peer-to-peer messaging between agents
on different Maidan deployments. Shipped in Cluster G (`maidan-a2a`,
`POST /a2a/v1/rpc` + `/a2a/v1/events`); see [[Capability Map]].

## Capability

A scoped permission token. Grants the bearer the right to perform a
specific set of actions for a bounded time. Shipped since Cluster F
(`maidan-auth`); the live vocabulary and route map are in [[Capability Map]].

## Tombstone

A row that marks an entity as deleted without physically removing it.
Used for audit, GDPR right-of-erasure, and reversible moderation.
