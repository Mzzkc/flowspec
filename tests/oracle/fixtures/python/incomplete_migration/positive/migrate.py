def deprecated_fetch(url):
    return "old:" + url

def fetch(url):
    return "new:" + url

def caller_one():
    return deprecated_fetch("http://a")

def caller_two():
    return deprecated_fetch("http://b")

def caller_three():
    return fetch("http://c")
