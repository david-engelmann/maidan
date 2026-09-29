------------------------------- MODULE Claim -------------------------------
(***************************************************************************)
(* The claim state machine of a thread queue: claim_next (with or without  *)
(* a lease), claim by id, assign, unassign, the fenced follow-ups (renew,  *)
(* acknowledge, release), a member freeze, time passing a lease deadline,  *)
(* and the reaper freeing a lapsed lease. Each action is one SQL transaction in the store, so it is one *)
(* atomic step here. claim_next may pick any claimable thread: the real    *)
(* query picks one of them, and every property is a safety property.      *)
(*                                                                         *)
(* A fencing token is issued once. A member knows the tokens it was        *)
(* handed; a fenced call presents one of them and changes nothing unless   *)
(* it is the thread's current (holder, token).                             *)
(*                                                                         *)
(* ResetDeadline: every write that changes the holder also writes the      *)
(* lease deadline (the fixed store). FALSE is the store before the fix,    *)
(* where release, assign and claim by id left the old deadline behind;     *)
(* ClaimNoReset.cfg checks that TLC finds that bug.                        *)
(***************************************************************************)
EXTENDS Naturals, FiniteSets

CONSTANTS Members, Threads, MaxToken, ResetDeadline

None == "none"

VARIABLES
    holder,     \* thread -> member or None
    token,      \* thread -> current fencing token (0: none)
    deadline,   \* thread -> "none" | "live" | "past"
    leased,     \* thread -> the current holder asked for a lease (ghost)
    working,    \* thread -> work_started_at is set
    known,      \* member -> set of <<thread, token>> it was handed
    issued,     \* tokens issued so far
    lost,       \* set of <<thread, member, token>> reported by ClaimExpired
    bad         \* a ClaimExpired for a holder whose lease had not lapsed (ghost)

vars == <<holder, token, deadline, leased, working, known, issued, lost, bad>>

Tokens == 1..MaxToken

TypeOK ==
    /\ holder \in [Threads -> Members \cup {None}]
    /\ token \in [Threads -> 0..MaxToken]
    /\ deadline \in [Threads -> {"none", "live", "past"}]
    /\ leased \in [Threads -> BOOLEAN]
    /\ working \in [Threads -> BOOLEAN]
    /\ known \in [Members -> SUBSET (Threads \X Tokens)]
    /\ issued \in 0..MaxToken
    /\ lost \subseteq Threads \X Members \X Tokens
    /\ bad \in BOOLEAN

Init ==
    /\ holder = [t \in Threads |-> None]
    /\ token = [t \in Threads |-> 0]
    /\ deadline = [t \in Threads |-> "none"]
    /\ leased = [t \in Threads |-> FALSE]
    /\ working = [t \in Threads |-> FALSE]
    /\ known = [m \in Members |-> {}]
    /\ issued = 0
    /\ lost = {}
    /\ bad = FALSE

Claimable(t) == holder[t] = None \/ deadline[t] = "past"

\* The deadline a write that changes the holder leaves behind.
Handover(t, d) == IF ResetDeadline THEN d ELSE deadline[t]

\* Hand member m a fresh token for thread t.
Grant(m, t) ==
    /\ issued < MaxToken
    /\ issued' = issued + 1
    /\ token' = [token EXCEPT ![t] = issued + 1]
    /\ known' = [known EXCEPT ![m] = @ \cup {<<t, issued + 1>>}]
    /\ holder' = [holder EXCEPT ![t] = m]
    /\ working' = [working EXCEPT ![t] = FALSE]

\* claim_next_thread: take a free thread or one whose lease lapsed. Taking
\* it from a holder emits ClaimExpired for that holder.
ClaimNext(m, lease) ==
    \E t \in Threads :
        /\ Claimable(t)
        /\ Grant(m, t)
        /\ deadline' = [deadline EXCEPT ![t] = IF lease THEN "live" ELSE "none"]
        /\ leased' = [leased EXCEPT ![t] = lease]
        /\ IF holder[t] # None
              THEN /\ lost' = lost \cup {<<t, holder[t], token[t]>>}
                   /\ bad' = (bad \/ ~leased[t] \/ deadline[t] # "past")
              ELSE UNCHANGED <<lost, bad>>

\* claim_thread: compare-and-set on an unassigned thread; no lease.
ClaimById(m, t) ==
    /\ holder[t] = None
    /\ Grant(m, t)
    /\ deadline' = [deadline EXCEPT ![t] = Handover(t, "none")]
    /\ leased' = [leased EXCEPT ![t] = FALSE]
    /\ UNCHANGED <<lost, bad>>

\* assign_thread: unconditional (assign / handoff); no lease.
Assign(m, t) ==
    /\ Grant(m, t)
    /\ deadline' = [deadline EXCEPT ![t] = Handover(t, "none")]
    /\ leased' = [leased EXCEPT ![t] = FALSE]
    /\ UNCHANGED <<lost, bad>>

Free(t) ==
    /\ holder' = [holder EXCEPT ![t] = None]
    /\ token' = [token EXCEPT ![t] = 0]
    /\ working' = [working EXCEPT ![t] = FALSE]
    /\ deadline' = [deadline EXCEPT ![t] = Handover(t, "none")]
    /\ leased' = [leased EXCEPT ![t] = FALSE]

\* unassign_thread: unconditional.
Unassign(t) ==
    /\ holder[t] # None
    /\ Free(t)
    /\ UNCHANGED <<known, issued, lost, bad>>

\* A member freeze returns every thread the member holds to the queue.
Freeze(m) ==
    /\ \E t \in Threads : holder[t] = m
    /\ holder' = [t \in Threads |-> IF holder[t] = m THEN None ELSE holder[t]]
    /\ token' = [t \in Threads |-> IF holder[t] = m THEN 0 ELSE token[t]]
    /\ working' = [t \in Threads |-> IF holder[t] = m THEN FALSE ELSE working[t]]
    /\ deadline' = [t \in Threads |-> IF holder[t] = m THEN "none" ELSE deadline[t]]
    /\ leased' = [t \in Threads |-> IF holder[t] = m THEN FALSE ELSE leased[t]]
    /\ UNCHANGED <<known, issued, lost, bad>>

\* The fence on renew, acknowledge and release.
Fenced(m, t, k) == holder[t] = m /\ token[t] = k

\* A fenced call with any token the member was ever handed. A stale token
\* (or a member that lost the thread) changes nothing.
Renew(m, t, k) ==
    /\ <<t, k>> \in known[m]
    /\ IF Fenced(m, t, k)
          THEN /\ deadline' = [deadline EXCEPT ![t] = "live"]
               /\ leased' = [leased EXCEPT ![t] = TRUE]
          ELSE UNCHANGED <<deadline, leased>>
    /\ UNCHANGED <<holder, token, working, known, issued, lost, bad>>

Acknowledge(m, t, k) ==
    /\ <<t, k>> \in known[m]
    /\ IF Fenced(m, t, k)
          THEN working' = [working EXCEPT ![t] = TRUE]
          ELSE UNCHANGED working
    /\ UNCHANGED <<holder, token, deadline, leased, known, issued, lost, bad>>

Release(m, t, k) ==
    /\ <<t, k>> \in known[m]
    /\ IF Fenced(m, t, k)
          THEN Free(t)
          ELSE UNCHANGED <<holder, token, working, deadline, leased>>
    /\ UNCHANGED <<known, issued, lost, bad>>

\* Time passes a live lease deadline.
Lapse(t) ==
    /\ deadline[t] = "live"
    /\ deadline' = [deadline EXCEPT ![t] = "past"]
    /\ UNCHANGED <<holder, token, leased, working, known, issued, lost, bad>>

\* The claim reaper: a lapsed lease is returned to the queue with nobody
\* calling claim_next, and ClaimExpired is reported for the holder.
Reap(t) ==
    /\ holder[t] # None
    /\ deadline[t] = "past"
    /\ holder' = [holder EXCEPT ![t] = None]
    /\ token' = [token EXCEPT ![t] = 0]
    /\ working' = [working EXCEPT ![t] = FALSE]
    /\ deadline' = [deadline EXCEPT ![t] = "none"]
    /\ leased' = [leased EXCEPT ![t] = FALSE]
    /\ lost' = lost \cup {<<t, holder[t], token[t]>>}
    /\ bad' = (bad \/ ~leased[t])
    /\ UNCHANGED <<known, issued>>

Next ==
    \/ \E m \in Members, lease \in BOOLEAN : ClaimNext(m, lease)
    \/ \E m \in Members, t \in Threads : ClaimById(m, t) \/ Assign(m, t)
    \/ \E t \in Threads : Unassign(t) \/ Lapse(t) \/ Reap(t)
    \/ \E m \in Members : Freeze(m)
    \/ \E m \in Members, t \in Threads, k \in Tokens :
          Renew(m, t, k) \/ Acknowledge(m, t, k) \/ Release(m, t, k)

Spec == Init /\ [][Next]_vars

-----------------------------------------------------------------------------
(* Invariants *)

\* A token is issued once: no two members, or threads, share one.
TokensUnique ==
    \A m1, m2 \in Members, t1, t2 \in Threads, k \in Tokens :
        (<<t1, k>> \in known[m1] /\ <<t2, k>> \in known[m2]) => (m1 = m2 /\ t1 = t2)

\* The thread's current token was handed to its holder, and only to it.
TokenBelongsToHolder ==
    \A t \in Threads :
        holder[t] # None =>
            /\ <<t, token[t]>> \in known[holder[t]]
            /\ \A m \in Members \ {holder[t]} : <<t, token[t]>> \notin known[m]

\* A free thread carries no token, deadline or working clock.
FreeIsClean ==
    \A t \in Threads :
        holder[t] = None => (token[t] = 0 /\ deadline[t] = "none" /\ ~working[t])

\* A holder that never asked for a lease has no deadline, so claim_next can
\* never take the thread from it.
NoLeaseNoDeadline ==
    \A t \in Threads : (holder[t] # None /\ ~leased[t]) => deadline[t] = "none"

\* ClaimExpired is reported only for a holder whose lease lapsed.
ExpiryOnlyForLapsedLeases == ~bad

\* A token lost to a takeover is fenced for good: it never passes again.
LostTokensStayFenced ==
    \A <<t, m, k>> \in lost : ~Fenced(m, t, k)

=============================================================================
