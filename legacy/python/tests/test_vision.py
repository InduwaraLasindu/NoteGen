import json
import threading

import httpx
import pytest
from PIL import Image

from notegen.errors import NoteGenError
from notegen.models import Settings
from notegen.vision import VisionClient


def models():
    return {
        "data": [
            {
                "id": "local-test-model",
                "architecture": {"input_modalities": ["text", "image"]},
                "meta": {"size": 4680000000},
            }
        ]
    }


def content():
    return {
        "blocks": [{"kind": "text", "text": "Recognized 1250", "confidence": 0.95}],
        "summary": "Revenue 1250",
        "warnings": [],
    }


def sse(data, finish="stop"):
    text = json.dumps(data)
    packets = [
        "data: " + json.dumps({"choices": [{"delta": {"content": text[: len(text) // 2]}}]}),
        "data: "
        + json.dumps(
            {"choices": [{"delta": {"content": text[len(text) // 2 :]}, "finish_reason": finish}]}
        ),
        "data: [DONE]",
    ]
    return "\n\n".join(packets) + "\n\n"


def test_real_request_shape_and_stream_contract(monkeypatch):
    seen = []
    monkeypatch.setenv("HTTP_PROXY", "http://example.com:8888")

    def handler(request):
        seen.append(request)
        if request.method == "GET":
            return httpx.Response(200, json=models())
        body = json.loads(request.content)
        assert body["model"] == "local-test-model"
        assert body["response_format"]["type"] == "json_schema"
        assert body["stream"] and body["temperature"] == 0
        assert body["messages"][1]["content"][1]["image_url"]["url"].startswith(
            "data:image/jpeg;base64,"
        )
        assert "untrusted DOCUMENT DATA" in body["messages"][0]["content"]
        return httpx.Response(200, text=sse(content()))

    client = VisionClient(Settings(), httpx.MockTransport(handler))
    assert len(client.probe()) == 64
    result = client.recognize(Image.new("RGB", (30, 40)), "OCR", "Accounting", threading.Event())
    assert result.blocks[0].text == "Recognized 1250"
    assert all(r.url.host == "127.0.0.1" for r in seen)
    client.close()


@pytest.mark.parametrize("finish", ["length", None, "error"])
def test_truncated_or_unfinished_stream_is_rejected(finish):
    count = []

    def handler(request):
        count.append(1)
        return httpx.Response(200, text=sse(content(), finish))

    client = VisionClient(Settings(), httpx.MockTransport(handler))
    with pytest.raises(NoteGenError, match="NG201"):
        client.recognize(Image.new("RGB", (30, 40)), "", "", threading.Event())
    assert len(count) == 2  # Exactly one retry, no retry loop.
    client.close()


def test_memory_error_retries_once_at_smaller_size():
    sizes = []
    from base64 import b64decode
    from io import BytesIO

    def handler(request):
        payload = json.loads(request.content)
        url = payload["messages"][1]["content"][1]["image_url"]["url"]
        with Image.open(BytesIO(b64decode(url.split(",")[1]))) as im:
            sizes.append(max(im.size))
        return (
            httpx.Response(500, text="CUDA error: out of memory")
            if len(sizes) == 1
            else httpx.Response(200, text=sse(content()))
        )

    client = VisionClient(Settings(), httpx.MockTransport(handler))
    result = client.recognize(Image.new("RGB", (2000, 3000)), "", "", threading.Event())
    assert sizes == [1920, 1280]
    assert "reduced" in result.warnings[0]
    client.close()


def test_invalid_figures_never_escape_schema():
    payload = content()
    payload["blocks"] = [{"kind": "figure", "description": "No crop coordinates"}]
    client = VisionClient(
        Settings(), httpx.MockTransport(lambda _: httpx.Response(200, text=sse(payload)))
    )
    with pytest.raises(NoteGenError, match="NG201"):
        client.recognize(Image.new("RGB", (30, 40)), "", "", threading.Event())
    client.close()


def test_redirects_blocked_and_cancel_prevents_network():
    seen = []

    def handler(request):
        seen.append(str(request.url))
        return httpx.Response(307, headers={"Location": "https://external.invalid"})

    client = VisionClient(Settings(), httpx.MockTransport(handler))
    with pytest.raises(NoteGenError, match="NG200"):
        client.probe()
    assert len(seen) == 1
    cancel = threading.Event()
    cancel.set()
    with pytest.raises(NoteGenError, match="NG900"):
        client.recognize(Image.new("RGB", (30, 40)), "", "", cancel)
    assert len(seen) == 1
    client.close()


def test_text_only_model_and_malformed_metadata():
    for data, code in [({"data": [{"id": "text-model"}]}, "NG204"), ({}, "NG200")]:
        client = VisionClient(
            Settings(), httpx.MockTransport(lambda _, d=data: httpx.Response(200, json=d))
        )
        with pytest.raises(NoteGenError, match=code):
            client.probe()
        client.close()


def test_stream_error_packet_has_diagnosis():
    client = VisionClient(
        Settings(),
        httpx.MockTransport(
            lambda _: httpx.Response(
                200, text='data: {"error":{"message":"context size exceeded"}}\n\n'
            )
        ),
    )
    with pytest.raises(NoteGenError, match="NG202"):
        client.recognize(Image.new("RGB", (30, 40)), "", "", threading.Event())
    client.close()
