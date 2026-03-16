# Planted incomplete migration: deprecated_fetch + fetch coexist
def deprecated_fetch(url):
    """Old sync fetch implementation."""
    import urllib.request
    return urllib.request.urlopen(url).read()

def fetch(url):
    """New async-ready fetch implementation."""
    import httpx
    return httpx.get(url).text

def handler_old_style():
    return deprecated_fetch("http://example.com")

def handler_new_style():
    return fetch("http://example.com")

def handler_also_old():
    return deprecated_fetch("http://example.com/api")
