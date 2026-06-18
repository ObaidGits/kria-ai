#!/usr/bin/env python3
"""
Forensic WebRTC client for the KRIA remote-desktop `/rd-signal` endpoint.

Connects like the phone does: recvonly video offer → answer → ICE → media.
Prints the exact stage reached so we can see where the pipeline stops.

  rd_webrtc_probe.py <ws_url>
"""
import asyncio
import json
import sys
import time

import websockets
from aiortc import RTCPeerConnection, RTCConfiguration, RTCIceServer, RTCSessionDescription
from aiortc.sdp import candidate_from_sdp

STAGES = {
    "ws_open": False,
    "offer_received": False,
    "answer_sent": False,
    "ice_state": "new",
    "conn_state": "new",
    "track_received": False,
    "frames": 0,
}


def log(msg):
    print(f"[probe {time.strftime('%H:%M:%S')}] {msg}", flush=True)


async def main(ws_url):
    pc = RTCPeerConnection(
        RTCConfiguration(iceServers=[RTCIceServer(urls="stun:stun.l.google.com:19302")])
    )

    @pc.on("connectionstatechange")
    async def on_conn():
        STAGES["conn_state"] = pc.connectionState
        log(f"connectionState={pc.connectionState}")

    @pc.on("iceconnectionstatechange")
    async def on_ice():
        STAGES["ice_state"] = pc.iceConnectionState
        log(f"iceConnectionState={pc.iceConnectionState}")

    @pc.on("track")
    def on_track(track):
        STAGES["track_received"] = True
        log(f"[STEP 14a] track received: kind={track.kind}")

        async def recv_frames():
            try:
                while True:
                    await track.recv()
                    STAGES["frames"] += 1
                    if STAGES["frames"] in (1, 5, 30):
                        log(f"[STEP 14] media frame #{STAGES['frames']}")
            except Exception as e:  # noqa: BLE001
                log(f"frame recv ended: {e!r}")

        asyncio.ensure_future(recv_frames())

    async with websockets.connect(ws_url, max_size=4 * 1024 * 1024) as ws:
        STAGES["ws_open"] = True
        log("[STEP 1a] ws open — waiting for server offer (server is offerer)")

        async def reader():
            async for raw in ws:
                try:
                    msg = json.loads(raw)
                except Exception:  # noqa: BLE001
                    continue
                t = msg.get("type")
                if t == "offer":
                    STAGES["offer_received"] = True
                    log("[STEP 10c] server offer received")
                    await pc.setRemoteDescription(
                        RTCSessionDescription(sdp=msg["sdp"], type="offer")
                    )
                    answer = await pc.createAnswer()
                    await pc.setLocalDescription(answer)
                    await ws.send(json.dumps({"type": "answer", "sdp": pc.localDescription.sdp}))
                    STAGES["answer_sent"] = True
                    log("[STEP 11] answer sent")
                elif t == "ice":
                    cand = msg.get("candidate", "")
                    try:
                        raw = cand.split(":", 1)[1] if cand.startswith("candidate:") else cand
                        c = candidate_from_sdp(raw)
                        c.sdpMLineIndex = msg.get("sdp_mline_index", 0)
                        c.sdpMid = "0"
                        await pc.addIceCandidate(c)
                    except Exception as e:  # noqa: BLE001
                        log(f"addIceCandidate failed: {type(e).__name__}: {e} | cand={cand!r}")
                elif t == "error":
                    log(f"SERVER ERROR: {msg.get('message')}")

        reader_task = asyncio.ensure_future(reader())
        for _ in range(150):
            await asyncio.sleep(0.1)
            if STAGES["frames"] >= 5:
                break
        reader_task.cancel()

    await pc.close()

    log("==== FORENSIC RESULT ====")
    for k, v in STAGES.items():
        log(f"  {k} = {v}")
    if STAGES["frames"] > 0:
        log("RESULT: SUCCESS — media frames received")
        sys.exit(0)
    else:
        last = "none"
        for stage in ["ws_open", "offer_received", "answer_sent", "track_received"]:
            if STAGES.get(stage):
                last = stage
        log(f"RESULT: FAIL — last successful stage = {last}, ice={STAGES['ice_state']}, conn={STAGES['conn_state']}")
        sys.exit(1)


if __name__ == "__main__":
    asyncio.run(main(sys.argv[1]))
