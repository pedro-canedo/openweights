"""Count the actual request body, including chunked uploads without Content-Length."""
from starlette.exceptions import HTTPException


class BodyLimit:
    def __init__(self, app):
        self.app = app

    async def __call__(self, scope, receive, send):
        if scope['type'] != 'http':
            return await self.app(scope, receive, send)
        limit = (258 if scope['path'] == '/api/studio/sources' else 34) * 1024**2
        size = 0

        async def bounded_receive():
            nonlocal size
            message = await receive()
            if message['type'] == 'http.request':
                size += len(message.get('body', b''))
                if size > limit:
                    raise HTTPException(413, 'O corpo da requisição excede o limite de upload.')
            return message

        await self.app(scope, bounded_receive, send)
