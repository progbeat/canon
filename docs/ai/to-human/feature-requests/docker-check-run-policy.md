# Docker Check Run Policy

It would help to clarify when agents should run the Docker `canon check`
variant during an active repair loop. Building the image is useful coverage but
expensive and noisy, so a human-owned rule for when to defer it would reduce
wasted time without weakening the final merge gate.
