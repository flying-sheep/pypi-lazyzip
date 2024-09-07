"""Lazy ZIP over HTTP."""

from __future__ import annotations

from dataclasses import dataclass
from types import MappingProxyType
from typing import IO, TYPE_CHECKING

__all__ = ["HTTPRangeRequestUnsupportedError"]

from bisect import bisect_left, bisect_right
from contextlib import contextmanager
from tempfile import NamedTemporaryFile
from zipfile import BadZipFile, ZipFile

from pip._internal.network.utils import response_chunks

if TYPE_CHECKING:
    from collections.abc import Generator, Mapping
    from types import TracebackType
    from typing import Self

    import httpx


CONTENT_CHUNK_SIZE = 10 * 1024
HEADERS: Mapping[str, str] = MappingProxyType({"Accept-Encoding": "identity"})


class HTTPRangeRequestUnsupportedError(Exception):
    """HTTP range request is not supported."""


@dataclass
class LazyZipOverHTTP(IO[bytes]):
    """File-like object mapped to a ZIP file over HTTP.

    This uses HTTP range requests to lazily fetch the file's content,
    which is supposed to be fed to ZipFile.  If such requests are not
    supported by the server, raise HTTPRangeRequestUnsupported
    during initialization.
    """

    url: str
    session: httpx.AsyncClient
    chunk_size: int = CONTENT_CHUNK_SIZE

    def __post_init__(self) -> None:
        self._length = 0
        self._file = NamedTemporaryFile()
        self._left: list[int] = []
        self._right: list[int] = []

    @property
    def mode(self) -> str:
        """Opening mode, which is always rb."""
        return "rb"

    @property
    def name(self) -> str:
        """Path to the underlying file."""
        return self._file.name

    def seekable(self) -> bool:
        """Return whether random access is supported, which is True."""
        return True

    def close(self) -> None:
        """Close the file."""
        self._file.close()

    @property
    def closed(self) -> bool:
        """Whether the file is closed."""
        return self._file.closed

    def read(self, size: int = -1) -> bytes:
        """Read up to size bytes from the object and return them.

        As a convenience, if size is unspecified or -1,
        all bytes until EOF are returned.  Fewer than
        size bytes may be returned if EOF is reached.
        """
        download_size = max(size, self.chunk_size)
        start, length = self.tell(), self._length
        stop = length if size < 0 else min(start + download_size, length)
        start = max(0, stop - download_size)
        self._download(start, stop - 1)
        return self._file.read(size)

    def readable(self) -> bool:
        """Return whether the file is readable, which is True."""
        return True

    def seek(self, offset: int, whence: int = 0) -> int:
        """Change stream position and return the new absolute position.

        Seek to offset relative position indicated by whence:
        * 0: Start of stream (the default).  pos should be >= 0;
        * 1: Current position - pos may be negative;
        * 2: End of stream - pos usually negative.
        """
        return self._file.seek(offset, whence)

    def tell(self) -> int:
        """Return the current position."""
        return self._file.tell()

    def truncate(self, size: int | None = None) -> int:
        """Resize the stream to the given size in bytes.

        If size is unspecified resize to the current position.
        The current stream position isn't changed.

        Return the new file size.
        """
        return self._file.truncate(size)

    def writable(self) -> bool:
        """Return False."""
        return False

    async def __aenter__(self) -> Self:
        head = await self.session.head(self.url, headers=HEADERS)
        head.raise_for_status()
        assert head.status_code == 200  # noqa: PLR2004, S101
        self._length = int(head.headers["Content-Length"])
        self.truncate(self._length)
        if "bytes" not in head.headers.get("Accept-Ranges", "none"):
            msg = "range request is not supported"
            raise HTTPRangeRequestUnsupportedError(msg)
        self._check_zip()
        self._file.__enter__()
        return self

    async def __aexit__(
        self,
        exc_type: type[BaseException] | None,
        exc: BaseException | None,
        tb: TracebackType | None,
    ) -> None:
        self._file.__exit__(exc_type, exc, tb)

    @contextmanager
    def _stay(self) -> Generator[None, None, None]:
        """Return a context manager keeping the position.

        At the end of the block, seek back to original position.
        """
        pos = self.tell()
        try:
            yield
        finally:
            self.seek(pos)

    def _check_zip(self) -> None:
        """Check and download until the file is a valid ZIP."""
        end = self._length - 1
        for start in reversed(range(0, end, self.chunk_size)):
            self._download(start, end)
            with self._stay():
                try:
                    # For read-only ZIP files, ZipFile only needs
                    # methods read, seek, seekable and tell.
                    ZipFile(self)
                except BadZipFile:
                    pass
                else:
                    break

    def _stream_response(
        self,
        start: int,
        end: int,
        base_headers: Mapping[str, str] = HEADERS,
    ) -> httpx.Response:
        """Return HTTP response to a range request from start to end."""
        headers = dict(base_headers)
        headers["Range"] = f"bytes={start}-{end}"
        # TODO: Get range requests to be correctly cached
        headers["Cache-Control"] = "no-cache"
        return self.session.get(self.url, headers=headers, stream=True)

    def _merge(
        self,
        start: int,
        end: int,
        left: int,
        right: int,
    ) -> Generator[tuple[int, int], None, None]:
        """Return a generator of intervals to be fetched.

        Args:
        ----
            start (int): Start of needed interval
            end (int): End of needed interval
            left (int): Index of first overlapping downloaded data
            right (int): Index after last overlapping downloaded data

        """
        lslice, rslice = self._left[left:right], self._right[left:right]
        i = start = min([start] + lslice[:1])
        end = max([end] + rslice[-1:])
        for j, k in zip(lslice, rslice):
            if j > i:
                yield i, j - 1
            i = k + 1
        if i <= end:
            yield i, end
        self._left[left:right], self._right[left:right] = [start], [end]

    def _download(self, start: int, end: int) -> None:
        """Download bytes from start to end inclusively."""
        with self._stay():
            left = bisect_left(self._right, start)
            right = bisect_right(self._left, end)
            for start, end in self._merge(start, end, left, right):
                response = self._stream_response(start, end)
                response.raise_for_status()
                self.seek(start)
                for chunk in response_chunks(response, self.chunk_size):
                    self._file.write(chunk)
