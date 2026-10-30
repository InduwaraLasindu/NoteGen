import json
import os
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

import pytest
from PIL import Image, ImageDraw, ImageFont

from notegen.models import Settings
from notegen.quality import distance, transcription_metrics
from notegen.vision import VisionClient


def test_exact_transcription_metrics():
    assert distance("kitten", "sitting") == 3
    assert distance("", "a") == 1
    assert transcription_metrics("a b c", "a  b\nc")["cer"] == 0
    assert transcription_metrics("a b c", "a b d")["wer"] == pytest.approx(1 / 3)
    with pytest.raises(ValueError):
        transcription_metrics("x" * 5001, "x")


def test_real_loopback_http_stream_integration():
    from test_vision import content, models, sse

    requests = []

    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *_):
            pass

        def do_GET(self):
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.end_headers()
            self.wfile.write(json.dumps(models()).encode())

        def do_POST(self):
            data = self.rfile.read(int(self.headers["Content-Length"]))
            requests.append(json.loads(data))
            self.send_response(200)
            self.send_header("Content-Type", "text/event-stream")
            self.end_headers()
            self.wfile.write(sse(content()).encode())

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    client = VisionClient(Settings(endpoint=f"http://127.0.0.1:{server.server_port}"))
    try:
        assert client.probe()
        result = client.recognize(
            Image.new("RGB", (80, 100)), "Revenue", "Accounting", threading.Event()
        )
        assert "1250" in result.blocks[0].text
        assert len(requests) == 1
    finally:
        client.close()
        server.shutdown()
        server.server_close()
        thread.join(2)


@pytest.mark.gpu
@pytest.mark.skipif(
    os.environ.get("NOTEGEN_RUN_GPU") != "1",
    reason="Opt-in: needs a running local vision model and target GPU",
)
def test_real_vision_model_table_and_contextual_diagram():
    image = Image.new("RGB", (1200, 1600), "white")
    d = ImageDraw.Draw(image)
    try:
        font = ImageFont.truetype("C:/Windows/Fonts/arial.ttf", 32)
    except OSError:
        font = ImageFont.load_default(size=32)
    d.text((80, 80), "Water cycle: evaporation leads to condensation.", font=font, fill="black")
    d.rectangle((100, 300, 460, 420), outline="black", width=4)
    d.text((120, 345), "Evaporation", font=font, fill="black")
    d.rectangle((700, 300, 1100, 420), outline="black", width=4)
    d.text((720, 345), "Condensation", font=font, fill="black")
    d.line((460, 360, 700, 360), fill="black", width=5)
    d.polygon([(700, 360), (675, 345), (675, 375)], fill="black")
    for y in (700, 800, 900):
        d.line((100, y, 1100, y), fill="black", width=4)
    for x in (100, 600, 1100):
        d.line((x, 700, x, 900), fill="black", width=4)
    for xy, text in [
        ((130, 735), "Item"),
        ((630, 735), "Amount"),
        ((130, 835), "Revenue"),
        ((630, 835), "1250"),
    ]:
        d.text(xy, text, font=font, fill="black")
    client = VisionClient(Settings(request_timeout=120))
    try:
        assert client.probe()
        result = client.recognize(
            image, "", "A school science worksheet about the water cycle.", threading.Event()
        )
        assert any(
            b.kind == "table" and any("1250" in c for row in b.rows for c in row)
            for b in result.blocks
        )
        assert any(
            b.kind == "figure"
            and "condensation" in (b.description + " " + b.interpretation).lower()
            and b.interpretation
            for b in result.blocks
        )
    finally:
        client.close()
