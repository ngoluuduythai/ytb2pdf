"""Transcribe an audio file with mlx-whisper (Apple Silicon).

Usage: python3 transcribe.py <audio_path> <model> <language>

Prints a JSON array of {"start": secs, "end": secs, "text": str} to stdout.
Needs: pip install mlx-whisper   (Apple Silicon only)
"""

import json
import sys


def main() -> None:
    audio_path, model, language = sys.argv[1], sys.argv[2], sys.argv[3]
    import mlx_whisper

    result = mlx_whisper.transcribe(
        audio_path, path_or_hf_repo=model, language=language, verbose=False
    )
    segments = []
    for s in result.get("segments", []):
        text = " ".join(str(s.get("text", "")).split())
        if text:
            segments.append(
                {
                    "start": float(s.get("start", 0.0)),
                    "end": float(s.get("end", 0.0)),
                    "text": text,
                }
            )
    sys.stdout.write(json.dumps(segments, ensure_ascii=False))


if __name__ == "__main__":
    main()
