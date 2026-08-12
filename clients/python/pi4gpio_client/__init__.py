"""Python client for the local pi4gpiod Unix-socket API."""

from .client import (
    DEFAULT_SOCKET_PATH,
    Pi4gpioClient,
    Pi4gpioConnectionError,
    Pi4gpioError,
)

__version__ = "0.1.1"
__all__ = [
    "Pi4gpioClient",
    "Pi4gpioError",
    "Pi4gpioConnectionError",
    "DEFAULT_SOCKET_PATH",
]
