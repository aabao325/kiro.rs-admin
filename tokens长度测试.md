──────── 短请求　HTTP 200
{
  "model": "claude-opus-4-8",
  "id": "msg_bdrk_j6bzny24ngl6gnbcm7sz4qyhtxh4mqmy37km4c3f46cxs4jad45a",
  "type": "message",
  "role": "assistant",
  "content": [
    {
      "type": "text",
      "text": "ok"
    }
  ],
  "stop_reason": "end_turn",
  "stop_sequence": null,
  "stop_details": null,
  "usage": {
    "input_tokens": 16,
    "cache_creation_input_tokens": 0,
    "cache_read_input_tokens": 0,
    "cache_creation": {
      "ephemeral_5m_input_tokens": 0,
      "ephemeral_1h_input_tokens": 0
    },
    "output_tokens": 4,
    "output_tokens_details": {
      "thinking_tokens": 0
    },
    "service_tier": "standard"
  },
  "context_management": {
    "applied_edits": []
  }
}

──────── 长请求(追加文本)　HTTP 200
{
  "model": "claude-opus-4-8",
  "id": "msg_bdrk_ejz6auw46kvisrst6ysbqncqzgw3luwalzklbfjghgwycrgcqjnq",
  "type": "message",
  "role": "assistant",
  "content": [
    {
      "type": "text",
      "text": "ok"
    }
  ],
  "stop_reason": "end_turn",
  "stop_sequence": null,
  "stop_details": null,
  "usage": {
    "input_tokens": 182,
    "cache_creation_input_tokens": 0,
    "cache_read_input_tokens": 0,
    "cache_creation": {
      "ephemeral_5m_input_tokens": 0,
      "ephemeral_1h_input_tokens": 0
    },
    "output_tokens": 4,
    "output_tokens_details": {
      "thinking_tokens": 0
    },
    "service_tier": "standard"
  },
  "context_management": {
    "applied_edits": []
  }
}

上面的是官方的正确的数据，下面是你的不正确的数据：

──────── 短请求　HTTP 200
{
    "model": "claude-opus-4-8",
    "id": "msg_bdrk_1z86evu9chlu3ub3f59ckzb54sc9ugb31cqfpteubnc1n9lk71g1",
    "type": "message",
    "role": "assistant",
    "content": [
        {
            "type": "text",
            "text": "ok"
        }
    ],
    "stop_reason": "end_turn",
    "stop_sequence": null,
    "stop_details": null,
    "usage": {
        "input_tokens": 1,
        "cache_creation_input_tokens": 1944,
        "cache_read_input_tokens": 4537,
        "cache_creation": {
            "ephemeral_5m_input_tokens": 1944,
            "ephemeral_1h_input_tokens": 0
        },
        "output_tokens": 1,
        "output_tokens_details": {
            "thinking_tokens": 0
        },
        "service_tier": "standard",
        "inference_geo": "not_available"
    },
    "context_management": {
        "applied_edits": []
    }
}

──────── 长请求(追加文本)　HTTP 200
{
    "model": "claude-opus-4-8",
    "id": "msg_bdrk_q1z8rzczlwl86vr77s9pbczvz388iyoygtnl2e642npj2hhe90tu",
    "type": "message",
    "role": "assistant",
    "content": [
        {
            "type": "text",
            "text": "ok"
        }
    ],
    "stop_reason": "end_turn",
    "stop_sequence": null,
    "stop_details": null,
    "usage": {
        "input_tokens": 1,
        "cache_creation_input_tokens": 1994,
        "cache_read_input_tokens": 4654,
        "cache_creation": {
            "ephemeral_5m_input_tokens": 1994,
            "ephemeral_1h_input_tokens": 0
        },
        "output_tokens": 1,
        "output_tokens_details": {
            "thinking_tokens": 0
        },
        "service_tier": "standard",
        "inference_geo": "not_available"
    },
    "context_management": {
        "applied_edits": []
    }
}