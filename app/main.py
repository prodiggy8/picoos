from fastapi import FastAPI, WebSocket, WebSocketDisconnect
import requests

app = FastAPI()

@app.get("/")
def read_root():
    return {"Hello": "World"}

@app.websocket("/shell")
async def websocket_shell(websocket: WebSocket):
    await websocket.accept()
    try:
        while True:
            data = await websocket.receive_text()

            if data == '\b':
                print('\b \b', end="", flush=True)
            else:
                print(data, end="", flush=True)

    except WebSocketDisconnect:
        print("\n[Server] Client disconnected.")
