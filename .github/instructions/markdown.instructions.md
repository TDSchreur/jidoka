---
description: 'Write readable CommonMark Markdown'
applyTo: '**/*.md'
---

# Markdown

- Use headings in order and separate headings, paragraphs, lists, and code blocks with blank lines.
- Use consistent list markers and indent nested content so it renders as intended.
- Give fenced code blocks a language identifier when one applies.
- Use descriptive link text and meaningful alternative text for informative images. Wrap bare URLs in angle brackets for CommonMark autolinks.
- In documentation (`README.md` files and `docs/`), link every mention of a skill by name to its `SKILL.md` with a relative path, for example [`jidoka-tdd`](../skills/jidoka-tdd/SKILL.md), so all skill references read the same. Leave names plain where Markdown links do not render, such as code blocks and Mermaid diagrams, and in slash commands like `/jidoka-full`, which are typed rather than opened.
- Keep formatting simple and check the rendered result when the structure is complex. Refer to the [CommonMark specification](https://spec.commonmark.org/) when needed.
