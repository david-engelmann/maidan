------------------------------ MODULE EventLog ------------------------------
(***************************************************************************)
(* The hash-chained event log with crypto-shredded message words, on an    *)
(* origin and one federation peer.                                         *)
(*                                                                         *)
(* The origin appends message events. posted/edited words are sealed under *)
(* the message's content key before the event is hashed; tombstoned        *)
(* destroys the key and every queued copy of the words (webhook, egress    *)
(* and mail outboxes) in the same transaction. A subject once shredded     *)
(* stays shredded: a later event about it is sealed under a throwaway key. *)
(*                                                                         *)
(* The origin sends any event, in any order and as often as it likes; an   *)
(* envelope carries the key only if it was live when the event was read.   *)
(* The network may drop, duplicate, delay and reorder envelopes. The peer  *)
(* skips an event it already ingested and, with ChainCheck, refuses one    *)
(* that does not extend the chain it verified (verify_peer_link).          *)
(*                                                                         *)
(* A row is never rewritten, so every node's chain verifies throughout.    *)
(* Hashes are abstract: an event's hash commits to its position and the    *)
(* exact payload appended.                                                 *)
(*                                                                         *)
(* EventLogUnordered.cfg sets ChainCheck = FALSE and checks that TLC finds *)
(* the resurrection it prevents: a tombstone ingested before its post      *)
(* shreds nothing, and the post then brings a live key.                    *)
(***************************************************************************)
EXTENDS Naturals, Sequences, FiniteSets

CONSTANTS Messages, MaxLog, ChainCheck

Origin == "origin"
Peer == "peer"
Nodes == {Origin, Peer}

Kinds == {"posted", "edited", "tombstoned"}

\* A content key is <<m, i>>: the key created for message m by the event at
\* index i of that node's log. Nobody holds <<m, 0>>; it stands for words
\* sealed under a key this node never had.
Keys == Messages \X (0..MaxLog)

VARIABLES
    log,       \* node -> sequence of [kind, msg, key, origin, hash]
    keys,      \* node -> message -> key row [s: "absent" | "live" | "shredded", k]
    outbox,    \* node -> keys of the queued copies of words (webhook, egress, mail)
    net,       \* set of envelopes [idx, withKey]
    verified,  \* the last origin index the peer verified
    tombs      \* node -> messages whose tombstone that node ingested

vars == <<log, keys, outbox, net, verified, tombs>>

\* The bytes an event's hash commits to.
Hash(i, kind, m, k) == <<i, kind, m, k>>

NoRow(m) == [s |-> "absent", k |-> <<m, 0>>]
LiveRow(key) == [s |-> "live", k |-> key]
ShreddedRow(m) == [s |-> "shredded", k |-> <<m, 0>>]
Shredded(r) == r.s = "shredded"
Absent(r) == r.s = "absent"
IsLive(r) == r.s = "live"
\* The live key of message m at node n, if any.
LiveKey(n, m, key) == IsLive(keys[n][m]) /\ keys[n][m].k = key

Init ==
    /\ log = [n \in Nodes |-> <<>>]
    /\ keys = [n \in Nodes |-> [m \in Messages |-> NoRow(m)]]
    /\ outbox = [n \in Nodes |-> {}]
    /\ net = {}
    /\ verified = 0
    /\ tombs = [n \in Nodes |-> {}]

\* Append one message event at node n as the store's append does
\* (content_keys::decide, shred_in_tx). `words` is FALSE for an event that
\* arrived sealed without its key. `from` is the origin index (0: local).
AppendAt(n, kind, m, words, from) ==
    LET i == Len(log[n]) + 1
        r == keys[n][m]
        \* [key the words are sealed under, key row after, shred_in_tx ran]
        d == CASE kind = "tombstoned" ->
                    \* shred_in_tx is an UPDATE: it shreds a row that exists.
                    [key |-> <<m, 0>>, row |-> IF Absent(r) THEN r ELSE ShreddedRow(m),
                     shred |-> TRUE]
               [] ~words /\ Absent(r) ->
                    [key |-> <<m, 0>>, row |-> ShreddedRow(m), shred |-> FALSE]
               [] ~words /\ IsLive(r) ->
                    [key |-> <<m, 0>>, row |-> ShreddedRow(m), shred |-> TRUE]
               [] ~words ->
                    [key |-> <<m, 0>>, row |-> r, shred |-> FALSE]
               [] Absent(r) ->
                    [key |-> <<m, i>>, row |-> LiveRow(<<m, i>>), shred |-> FALSE]
               [] Shredded(r) ->
                    \* A throwaway key, never stored.
                    [key |-> <<m, i>>, row |-> r, shred |-> FALSE]
               [] OTHER ->
                    [key |-> r.k, row |-> r, shred |-> FALSE]
    IN /\ log' = [log EXCEPT ![n] =
                     Append(@, [kind |-> kind, msg |-> m, key |-> d.key,
                                origin |-> from, hash |-> Hash(i, kind, m, d.key)])]
       /\ keys' = [keys EXCEPT ![n][m] = d.row]
       /\ outbox' = [outbox EXCEPT ![n] =
                        IF d.shred THEN {c \in @ : c[1] # m}
                        ELSE IF IsLive(d.row) /\ d.row.k = d.key THEN @ \cup {d.key}
                        ELSE @]
       /\ tombs' = IF kind = "tombstoned"
                      THEN [tombs EXCEPT ![n] = @ \cup {m}]
                      ELSE tombs

\* The origin writes an event: a message is posted once, then edited or
\* tombstoned any number of times (an edit after the tombstone must stay
\* unreadable).
Write(kind, m) ==
    /\ Len(log[Origin]) < MaxLog
    /\ (kind = "posted") = Absent(keys[Origin][m])
    /\ AppendAt(Origin, kind, m, TRUE, 0)
    /\ UNCHANGED <<net, verified>>

\* The origin reads event i and sends it, opened with its key if the key is
\* still live.
Send(i) ==
    /\ i \in 1..Len(log[Origin])
    /\ LET e == log[Origin][i]
       IN net' = net \cup {[idx |-> i,
                            withKey |-> e.kind # "tombstoned"
                                        /\ LiveKey(Origin, e.msg, e.key)]}
    /\ UNCHANGED <<log, keys, outbox, verified, tombs>>

Ingested == {log[Peer][j].origin : j \in 1..Len(log[Peer])}

\* The peer ingests an envelope: skip a duplicate, verify the chain, append.
Deliver(env) ==
    /\ env \in net
    /\ env.idx \notin Ingested
    /\ ChainCheck => env.idx = verified + 1
    /\ verified' = IF env.idx > verified THEN env.idx ELSE verified
    /\ LET e == log[Origin][env.idx]
       IN AppendAt(Peer, e.kind, e.msg, env.withKey, env.idx)
    /\ UNCHANGED net

Next ==
    \/ \E kind \in Kinds, m \in Messages : Write(kind, m)
    \/ \E i \in 1..MaxLog : Send(i)
    \/ \E env \in net : Deliver(env)

Spec == Init /\ [][Next]_vars

-----------------------------------------------------------------------------
(* Invariants *)

TypeOK ==
    /\ \A n \in Nodes : \A m \in Messages :
          keys[n][m] \in [s : {"absent", "live", "shredded"}, k : Keys]
    /\ \A n \in Nodes : outbox[n] \subseteq Keys
    /\ \A n \in Nodes : tombs[n] \subseteq Messages
    /\ verified \in 0..MaxLog

\* Words are readable at n exactly when n holds the live key they were
\* sealed under.
Readable(n, j) ==
    LET e == log[n][j] IN e.kind # "tombstoned" /\ LiveKey(n, e.msg, e.key)

\* Once a node ingested a message's tombstone, no copy of its words there is
\* readable, and no queued delivery still carries them.
ShreddedIsUnreadable ==
    \A n \in Nodes :
        /\ \A j \in 1..Len(log[n]) : log[n][j].msg \in tombs[n] => ~Readable(n, j)
        /\ \A c \in outbox[n] : c[1] \notin tombs[n]

\* The chain still verifies after any shred: every row is exactly what was
\* hashed when it was appended. No action rewrites a row, so this holds by
\* construction; it pins that shredding changes key rows, never log rows.
ChainVerifies ==
    \A n \in Nodes : \A j \in 1..Len(log[n]) :
        LET e == log[n][j] IN e.hash = Hash(j, e.kind, e.msg, e.key)

\* With ChainCheck the peer holds a prefix of the origin's log, in order.
PeerIsOrderedPrefix ==
    ChainCheck =>
        \A j \in 1..Len(log[Peer]) :
            /\ log[Peer][j].origin = j
            /\ log[Peer][j].kind = log[Origin][j].kind
            /\ log[Peer][j].msg = log[Origin][j].msg

\* A shredded key never comes back (action property).
ShreddingIsIrreversible ==
    [][\A n \in Nodes, m \in Messages :
          Shredded(keys[n][m]) => Shredded(keys'[n][m])]_vars

=============================================================================
