#!/usr/bin/env python3
"""联调调试快照：dump 安装链步骤 / 消息流 / 关键 UI 状态。"""
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from it_layer2_gui import js  # noqa: E402

EXPR = """(() => {
  const steps = [...document.querySelectorAll('.install-step')].map(x => x.innerText.split(String.fromCharCode(10)).join(' | ').slice(0, 70));
  const feed = [...document.querySelectorAll('.msg-feed .msg-text')].map(x => x.innerText).slice(-6);
  const chips = [...document.querySelectorAll('.status-chip')].map(x => x.innerText).slice(0, 20);
  return JSON.stringify({
    steps: steps,
    feed: feed,
    chips: chips,
    hasListen: document.body.innerText.includes('实测监听'),
    hasHello: document.body.innerText.includes('hello'),
    nodeLabel: (document.querySelector('.view-head__title h2') || {}).innerText || ''
  });
})()"""

if __name__ == "__main__":
    print(js(EXPR))
