#!/usr/bin/env python3
"""The test server hands out dashboard files only; run with python3 tests/dashboard-assets.py (needs no browser)."""
import http.client
import http.server
import threading
import unittest

from dashboard_browser import Handler


class AssetsTest(unittest.TestCase):
    def status(self, path):
        server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        thread = threading.Thread(target=server.serve_forever)
        thread.start()
        try:
            connection = http.client.HTTPConnection("127.0.0.1", server.server_port)
            connection.request("GET", path)
            return connection.getresponse().status
        finally:
            server.shutdown()
            thread.join()
            server.server_close()

    def test_serves_a_dashboard_file(self):
        self.assertEqual(self.status("/assets/app.js"), 200)
        self.assertEqual(self.status("/assets/app.js?v=1#top"), 200)

    def test_refuses_paths_outside_the_dashboard_folder(self):
        for path in ["/assets//etc/passwd", "/assets/../README.md", "/assets/../../etc/passwd", "/assets/missing.js", "/assets/app.js%00", "/assets/..%2f..%2fREADME.md"]:
            self.assertEqual(self.status(path), 404, path)


if __name__ == "__main__":
    unittest.main()
