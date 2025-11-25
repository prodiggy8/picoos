import asyncio
import websockets
import sys
import os

SERVER_URI = "ws://localhost:8080/shell"

# --- Cross-Platform Single Character Input ---
if os.name == 'nt':  # Windows
    import msvcrt
    def get_char():
        return msvcrt.getch().decode('utf-8')
else:  # Unix/Linux/macOS
    import tty
    import termios
    def get_char():
        fd = sys.stdin.fileno()
        old_settings = termios.tcgetattr(fd)
        try:
            tty.setraw(sys.stdin.fileno())
            ch = sys.stdin.read(1)
        finally:
            termios.tcsetattr(fd, termios.TCSADRAIN, old_settings)
        return ch

async def start_client():
    print(f"Connecting to {SERVER_URI}...")
    print("Sending keystrokes blindly to server... (Press Ctrl+C to exit)")
    
    try:
        async with websockets.connect(SERVER_URI) as websocket:
            loop = asyncio.get_running_loop()

            while True:
                # 1. Read character (Blocking, but in thread)
                char = await loop.run_in_executor(None, get_char)

                # 2. Handle Ctrl+C to exit
                if char == '\x03':
                    break

                # 3. Normalize Backspace keys to standard '\b'
                # Windows sends \x08, Unix sends \x7f
                if char in ('\x08', '\x7f'):
                    await websocket.send('\b')
                    continue

                # 4. Normalize Enter key to standard '\n'
                # Raw mode usually reads Enter as \r
                if char == '\r':
                    await websocket.send('\n')
                    continue

                # 5. Send normal character
                await websocket.send(char)

    except Exception as e:
        print(f"\n[Error] {e}")

if __name__ == "__main__":
    try:
        asyncio.run(start_client())
    except KeyboardInterrupt:
        pass
