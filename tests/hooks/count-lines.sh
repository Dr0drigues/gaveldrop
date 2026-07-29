#!/bin/sh
lines=$(jq -r '.stdout' | grep -c .)
if [ "$lines" -eq 3 ]; then
  printf '{"ok":true,"diffs":[]}'
else
  printf '{"ok":false,"diffs":[{"path":"expect.exec.lines","expected":"3","got":"%s"}]}' "$lines"
fi
