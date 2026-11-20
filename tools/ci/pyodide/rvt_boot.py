"""Stage the model file at the path the notebook reads, in the Pyodide file system.

The notebook opens a path, so in the browser the file is first fetched into Pyodide's in-memory file system
(synchronous request; marimo runs the notebook in a worker). The URL must allow cross-origin reads (CORS) unless
it is served by the same host as the page."""
import os

import js


def stage(path, url):
    request = js.XMLHttpRequest.new()
    request.open("GET", url, False)
    request.responseType = "arraybuffer"
    request.send()
    if request.status != 200:
        raise OSError(f"{url}: HTTP {request.status}")
    os.makedirs(os.path.dirname(path) or ".", exist_ok=True)
    with open(path, "wb") as f:
        f.write(js.Uint8Array.new(request.response).to_py())
    return path
