def fetch(url):
    return "new:" + url

def caller_one():
    return fetch("http://a")

def caller_two():
    return fetch("http://b")
