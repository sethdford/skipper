#!/usr/bin/env python3
"""Example Skipper agent: echoes back messages with a friendly greeting."""

import sys
import os

# Add parent directory to path for skipper_sdk import
sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))
from skipper_sdk import Agent

agent = Agent()


@agent.on_message
def handle(message: str, context: dict) -> str:
    agent_id = context.get("agent_id", os.environ.get("SKIPPER_AGENT_ID", "unknown"))
    return f"Hello from Python agent {agent_id}! You said: {message}"


agent.run()
