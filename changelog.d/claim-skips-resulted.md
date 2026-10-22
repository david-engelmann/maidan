### claim_next no longer hands out finished work whose claim lapsed

An open thread that already has a stored result is skipped by `claim_next` on
both backends and in both scopes (channel and workspace). A reviewer sending it
back (review to open after the result) is the explicit re-queue that makes it
claimable again.
