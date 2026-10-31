### Find-by-attribute reads the author family

- **Fixed:** the find-by-attribute row reader (#1751) and the batch that added `author_family` (#1752) merged separately, and `dev` stopped compiling: the reader built `TaskAttributes` without the new field. It now reads `author_family`, and both backends' search queries select it.
