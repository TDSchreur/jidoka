## Changed

- The workflow procedures now list `jidoka-implement` among the skills only the user can invoke, so the Full workflow reads it instead of calling a skill the agent may not invoke.
- `jidoka-code-review` asks its review agents for brief findings without preamble instead of a 400-word limit.
- `jidoka-to-spec` asks for user stories that cover the actors and behavior-changing edge cases, without restating one another, instead of an "extremely extensive" list.
- `jidoka-security-review` no longer repeats its trigger list in the skill body.
