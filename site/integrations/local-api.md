# Local API server

**Local Server** exposes an OpenAI-compatible API so other apps can use the
model you already have loaded.

## Starting it

Set the port, press **Start**, and the address appears with a copy button.
Everything below applies at start time — change a setting and the engine has to
be stopped and started again for it to take effect. The screen says so, and it
refuses to restart while an agent run or a project indexing is using the engine
(it names which one).

| Setting | What it does |
|---|---|
| **Port** | Where it listens |
| **Allow access from local network** | Other devices on your network can reach the API |
| **API key** | Optional; when set, requests must present it |
| **Concurrent models** | With 1, switching models unloads the previous one — what most GPUs can take. Above that, models stay loaded together and may not fit in video memory |
| **Conversations at once** | Each simultaneous conversation takes a slice of the context window. With 1, the window you ask for is the window you get |

The screen also lists the models currently in memory, and the server log.

## Using it

::: code-group

```bash [curl]
curl http://127.0.0.1:PORT/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{"model": "YOUR-MODEL", "messages": [{"role": "user", "content": "Hello!"}]}'
```

```python [Python]
from openai import OpenAI

client = OpenAI(base_url="http://127.0.0.1:PORT/v1", api_key="local")
resp = client.chat.completions.create(
    model="YOUR-MODEL",
    messages=[{"role": "user", "content": "Hello!"}],
)
print(resp.choices[0].message.content)
```

:::

`GET /v1/models` lists what is loaded. Model ids are the ones shown in **My
Models**.

::: warning Local network means local network
Turning on network access removes the "only this machine" boundary. Anyone on
the same network reaches your models — set an API key, and only enable it on a
network you trust.
:::

## Requirements

The AI engine has to be installed first — that happens on your first run. Until
then the screen says so rather than failing on start.
