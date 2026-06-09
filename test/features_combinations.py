#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 The rsinit Authors
# SPDX-License-Identifier: GPL-2.0-only

import itertools
import json
import subprocess

data = json.loads(subprocess.check_output(["cargo", "metadata", "--no-deps"]))
features = list(data["packages"][0]["features"].keys())
features.remove("default")

for x in [
    ",".join(x)
    for i in range(len(features))
    for x in itertools.combinations(features, i)
]:
    print(x)
