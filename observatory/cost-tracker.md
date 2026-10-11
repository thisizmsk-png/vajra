# Cost Tracker

Cost in v0.5 is measured from the session transcript, not estimated during a run. The
removed `cost.ratesPer1MTokens` and `cost.alertThreshold` config keys no longer exist;
`scripts/vajra-cost.sh` is the tool, and current per-model prices live in
`config/pricing.json`.

## How to track

Run the cost script over a transcript (or `/vajra cost`):

```
scripts/vajra-cost.sh [transcript.jsonl]
```

It reads real token usage from the transcript, dedupes by message id so a resumed or
replayed turn is not double-counted, matches each message's model to a rate in
`config/pricing.json` (most specific match first), and prints input/output tokens and
the USD total.

## Pricing

`config/pricing.json` holds the current per-1M-token input/output rates (opus-5-5,
sonnet-5-5, haiku-5-5, fable-5-1, plus legacy entries for older transcripts). Update
prices there; the script does not hard-code any rate.

## Routing cost

The router (`scripts/vajra_route.py`) is a 0-token `UserPromptSubmit` hook: it adds a
suggestion without a model call. A bare `/vajra <ask>` then launches the `vajra-deliver`
workflow, whose many agents are where the tokens actually go — watch a large run's spend
in `/workflows`, which shows each agent's token usage as it progresses, and stop it there
if needed.
