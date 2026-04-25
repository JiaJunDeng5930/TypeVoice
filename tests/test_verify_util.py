import pytest

from scripts import _verify_util


@pytest.mark.quick
def test_ffmpeg_preprocess_args_keep_asr_input_format():
    args = _verify_util._ffmpeg_preprocess_args("in.ogg", "out.wav")

    assert "-ac" in args
    assert args[args.index("-ac") + 1] == "1"
    assert "-ar" in args
    assert args[args.index("-ar") + 1] == "16000"
    assert "-c:a" in args
    assert args[args.index("-c:a") + 1] == "pcm_s16le"
    assert args[-1] == "out.wav"
