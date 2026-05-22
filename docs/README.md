# Maidan documentation

An [Obsidian](https://obsidian.md/) vault holding the design, roadmap,
and conventions for Maidan. Open this folder as a vault in Obsidian for
wikilink navigation, graph view, and backlinks.

## Index

- [[Architecture]] — high-level design.
- [[Roadmap]] — clusters from foundation to v1.0.
- [[Conventions]] — branch, commit, and PR conventions.
- [[Glossary]] — domain vocabulary.
- [[Capabilities]] — running list of what Maidan can do, by release.
- [[Clusters/Cluster A]] — current cluster: foundation.
- [[Retros/README]] — closing-wave retrospectives, one per cluster.

## Layout

```
docs/
├── README.md           this file
├── Architecture.md
├── Roadmap.md
├── Conventions.md
├── Glossary.md
├── Capabilities.md
├── Clusters/
│   └── Cluster A.md
└── Retros/
    └── README.md
```

## Conventions inside the vault

- Wikilinks (`[[Note Name]]`) for internal references.
- Filenames use Title Case with spaces; Obsidian resolves them.
- Diagrams use [Mermaid](https://mermaid.js.org/) inside fenced code blocks.
- Each note begins with a one-paragraph summary so the graph hover
  surfaces the right context.
