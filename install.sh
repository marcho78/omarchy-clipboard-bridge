#!/bin/bash
# Guest installer shortcut: run inside the Omarchy VM.
exec "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/bin/omarchy-clipboard-bridge" install
