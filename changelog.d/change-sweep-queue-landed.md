### A pull request a merge queue squashed and closed lands its task

- **Fixed:** a merge queue that squashes a pull request into a commit of its own on the base and then closes it (Graphite's does, so GitHub reports the pull request unmerged) left the task `in_review` with a "closed without merging" notice. The change sweep now counts the close as a landing when the base has a commit from the day before the close whose subject ends `(#N)`, and records that commit for deploy tracking. A revert or a passing mention does not count.
