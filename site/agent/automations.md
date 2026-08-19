# Automations

An automation is a request that runs on its own every so often, even when you
are not watching.

**Settings → Automations → New automation**:

| Field | What it is |
|---|---|
| Name | How it shows up in the list |
| What to do | The prompt, exactly as you would type it in the chat |
| Project folder | Where it runs; optional |
| Model | Or leave empty for the first one in your library |
| Authorization and work mode | The same levels as a normal run |
| How often | Hourly, every 4 hours, daily, weekly, a custom interval in minutes, or only when you ask |

## What runs unattended actually means

Nobody is watching when an automation fires. So the harness does the boring,
correct thing: on any level short of automatic, it **stops at the first
confirmation request and waits for you**. The list shows *Stopped, waiting for
you*, and opening the run lets you answer and let it continue.

The same applies to the other stops: *Stopped at the step limit*, *Stopped after
repeated errors*.

::: warning The clock lives inside the app
With OpenWeights closed, nothing fires. At the scheduled time the AI engine
starts on its own — but the app has to be running for the clock to tick.
:::

**Run now** fires one immediately, which is also how you test the prompt before
trusting it to a schedule. Turning an automation off keeps it in the list
without running it.

Every automation run lands in **Activity** like any other run, with its full
trail.
