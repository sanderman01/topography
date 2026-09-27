"""Loopback HTTPS WorldCover/GLIMS/Overpass fixtures for game cover checks (EPSG:32619 near x=700000)."""

import functools
import http.server
import json
import math
import os
from pathlib import Path
import ssl
import subprocess
import threading
import tomllib

from check_acquisition import ROOT, run

# WorldCover fixture extent: 3 km square at 10 m around every synthetic check raster.
LEFT, TOP, CELLS = 699000, 2002000, 300


class StaticHandler(http.server.SimpleHTTPRequestHandler):
    """Static files with byte ranges for GDAL; query strings are ignored."""

    def log_message(self, *_args):
        pass

    def send_head(self):
        path = Path(self.translate_path(self.path.split("?", 1)[0]))
        if not path.is_file():
            self.send_error(404)
            return None
        data = path.read_bytes()
        start, end = 0, len(data) - 1
        ranged = self.headers.get("Range", "").startswith("bytes=")
        if ranged:
            first, _, last = self.headers["Range"].removeprefix("bytes=").partition("-")
            start = int(first)
            end = min(int(last), len(data) - 1) if last else len(data) - 1
        self.send_response(206 if ranged else 200)
        self.send_header("Content-Length", str(end - start + 1))
        self.send_header("Accept-Ranges", "bytes")
        if ranged:
            self.send_header("Content-Range", f"bytes {start}-{end}/{len(data)}")
        self.end_headers()
        self._body = data[start:end + 1]
        return self

    def do_GET(self):
        if self.send_head():
            self.wfile.write(self._body)

    def do_HEAD(self):
        self.send_head()


def grid(path, width, height, left, top, res, value, *, srs="EPSG:32619"):
    """Byte raster whose cell value is value(x_centre, y_centre)."""
    asc = path.with_suffix(".asc")
    rows = [" ".join(str(value(left + (c + 0.5) * res, top - (r + 0.5) * res)) for c in range(width))
            for r in range(height)]
    asc.write_text(f"ncols {width}\nnrows {height}\nxllcorner {left}\nyllcorner {top - height * res}\n"
                   f"cellsize {res}\n" + "\n".join(rows) + "\n")
    run("gdal_translate", "-q", "-ot", "Byte", "-a_srs", srs, "-a_nodata", "none", asc, path)
    asc.unlink()
    return path


def lonlat(points):
    result = subprocess.run(["gdaltransform", "-s_srs", "EPSG:32619", "-t_srs", "EPSG:4326",
                             "-output_xy"], input="\n".join(f"{x} {y}" for x, y in points),
                            capture_output=True, text=True, check=True)
    return [[float(v) for v in line.split()[:2]] for line in result.stdout.splitlines()]


def outline(glac_id, anlys_id, date, box, line_type="glac_bound"):
    x0, y0, x1, y1 = box
    ring = lonlat([(x0, y0), (x1, y0), (x1, y1), (x0, y1), (x0, y0)])
    return {"type": "Feature", "id": f"outline.{anlys_id}",
            "geometry": {"type": "Polygon", "coordinates": [ring]},
            "properties": {"line_type": line_type, "anlys_id": anlys_id, "glac_id": glac_id,
                           "rec_status": "okay", "src_date": f"{date}T00:00:00Z"}}


def osm_way(way_id, points, **tags):
    """Overpass `out geom` way from EPSG:32619 points."""
    return {"type": "way", "id": way_id, "tags": tags,
            "geometry": [{"lat": lat, "lon": lon} for lon, lat in lonlat(points)]}


def osm_relation(rel_id, members, **tags):
    """Multipolygon relation from (role, points) member ways."""
    return {"type": "relation", "id": rel_id, "tags": dict(tags, type="multipolygon"),
            "members": [{"type": "way", "ref": i, "role": role,
                         "geometry": [{"lat": lat, "lon": lon} for lon, lat in lonlat(points)]}
                        for i, (role, points) in enumerate(members)]}


def tile_name():
    (lon, lat), = lonlat([(LEFT + CELLS * 5, TOP - CELLS * 5)])
    la, lo = math.floor(lat / 3) * 3, math.floor(lon / 3) * 3
    return f"{'S' if la < 0 else 'N'}{abs(la):02}{'W' if lo < 0 else 'E'}{abs(lo):03}"


class CoverServer:
    """Serve named scenarios; policy() returns an environment selecting one."""

    def __init__(self, directory):
        directory.mkdir(parents=True, exist_ok=True)
        self.root = directory / "root"
        self.root.mkdir()
        cert, key = directory / "cert.pem", directory / "key.pem"
        run("openssl", "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-days", "1", "-keyout", key,
            "-out", cert, "-subj", "/CN=localhost", "-addext", "subjectAltName=DNS:localhost")
        self.server = http.server.ThreadingHTTPServer(
            ("127.0.0.1", 0), functools.partial(StaticHandler, directory=str(self.root)))
        context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        context.load_cert_chain(cert, key)
        self.server.socket = context.wrap_socket(self.server.socket, server_side=True)
        threading.Thread(target=self.server.serve_forever, daemon=True).start()
        self.base = f"https://localhost:{self.server.server_port}"
        self.env = dict(os.environ, CURL_CA_BUNDLE=str(cert), NO_PROXY="localhost,127.0.0.1",
                        no_proxy="localhost,127.0.0.1")

    def policy(self, name, *, worldcover=lambda _x, _y: 30, glims=(), osm=()):
        """worldcover=None: no tile (outside coverage); glims/osm=None: that source returns 404.
        osm lists Overpass elements; the fixture ignores the query string."""
        folder = self.root / name
        folder.mkdir()
        if worldcover:
            grid(folder / f"wc-{tile_name()}.tif", CELLS, CELLS, LEFT, TOP, 10, worldcover)
        if glims is not None:
            (folder / "ows").write_text(json.dumps({"type": "FeatureCollection", "features": list(glims),
                                                    "numberMatched": len(glims)}))
        if osm is not None:
            (folder / "interpreter").write_text(json.dumps({"version": 0.6, "elements": list(osm)}))
        text = (ROOT / "configs/cover-game-v5.toml").read_text()
        config = tomllib.loads(text)
        for old, new in [(config["worldcover"]["url_template"], f"{self.base}/{name}/wc-{{tile}}.tif"),
                         (config["glims"]["endpoint"], f"{self.base}/{name}/ows"),
                         (config["osm"]["endpoint"], f"{self.base}/{name}/interpreter")]:
            text = text.replace(f'"{old}"', f'"{new}"')
        text = text.replace('source = "geofabrik"', 'source = "overpass"')
        path = self.root.parent / f"{name}-policy.toml"
        path.write_text(text)
        return dict(self.env, TOPOGRAPHY_COVER_POLICY=str(path))

    def close(self):
        self.server.shutdown()
        self.server.server_close()
