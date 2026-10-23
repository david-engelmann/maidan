### Queue-counts test writes its result under a claim

- **Fixed:** the shared-claimability test wrote a thread result without a claim lease, so main stopped compiling after the strict result fence landed. It now writes through the claiming test helper.
