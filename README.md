# lazy unzip over HTTPS

A toy example of how pip and uv read individual files from a remote zip archive.

Prerequisites are that the server supports range requests and optimally HTTP/2.

Current usage: `pypi-lazyzip (distname[==version]|path/to/dist.whl)`

An actual real-world use case would use connection pooling to process many wheels at the same time.
