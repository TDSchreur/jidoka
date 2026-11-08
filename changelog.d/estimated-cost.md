## Added

- Estimated cost of token usage. `factory run stats` and the dashboard run detail show a cost per row, model, Stage and run from a built-in price table (USD per million tokens, input, output, cache read and cache write), and the dashboard shows the cost beside the fresh tokens and cache reads per model. The cost is computed when shown and not stored. A model without a price is `unknown` (`null` in the API), never zero, and totals that include one are marked partial. The table lists the Claude models with a verified price; other models, including OpenAI and Copilot models, show as unknown.
- In `factory run stats` the Cache read column now sits next to Fresh.
