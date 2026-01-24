#!/usr/bin/env python3
import argparse
from http.server import BaseHTTPRequestHandler, HTTPServer
import threading

class PostbackHandler(BaseHTTPRequestHandler):
    def do_POST(self):
        length = int(self.headers.get("Content-Length", "0"))
        body = self.rfile.read(length)
        with open(self.server.output_path, "wb") as f:
            f.write(body)
        self.send_response(200)
        self.send_header("Content-Type", "text/plain")
        self.end_headers()
        self.wfile.write(b"ok")
        threading.Thread(target=self.server.shutdown, daemon=True).start()

    def log_message(self, format, *args):
        return


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=9911)
    parser.add_argument("--output", required=True)
    args = parser.parse_args()

    server = HTTPServer((args.host, args.port), PostbackHandler)
    server.output_path = args.output
    server.serve_forever()


if __name__ == "__main__":
    main()
