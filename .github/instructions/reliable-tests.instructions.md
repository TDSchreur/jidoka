---
description: 'Keep tests deterministic and independent across platforms'
applyTo: '**/*.rs,tests/**'
---

# Reliable tests

- Wait for an event or condition with a deadline instead of using a fixed sleep. Give failures a useful explanation.
- Use an OS-assigned port (`127.0.0.1:0`) for test servers and retrieve the assigned address.
- Give each test its own temporary files and other mutable state. Do not depend on test order or inherited environment settings.
- Stop and wait for any process or server started by a test, including when the test fails.
- Use portable paths and avoid assumptions about platform-specific signals, permissions, or timing. When a test cannot run on a platform, state why.
- Diagnose intermittent failures and fix their cause rather than hiding them with retries or longer timeouts.
