#!/bin/bash
# Test script to verify docker satisfaction detection

echo "Testing docker satisfaction detection..."
echo ""
echo "Query: 'Show all docker containers running on my host machine'"
echo ""

# Run a simple test
cargo test -p kria-core satisfaction_detects_docker_inspection -- --nocapture

echo ""
echo "If the test passes, satisfaction detection is working correctly."
echo "The issue might be:"
echo "1. LLM is returning multiple tool calls in one round"
echo "2. Satisfaction is detected but loop continues anyway"
echo "3. The goal text in TurnMemory doesn't match the pattern"
echo ""
echo "To debug further, check the KRIA logs for:"
echo "  - '🎯 SATISFACTION DETECTED' messages"
echo "  - '🛑 SKIPPING TOOL' messages"
echo "  - The actual goal text stored in TurnMemory"
