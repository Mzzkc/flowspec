import functools

def my_decorator(func):
    @functools.wraps(func)
    def wrapper(*args, **kwargs):
        return func(*args, **kwargs)
    return wrapper

@my_decorator
@staticmethod
def decorated_function():
    pass

class MyClass:
    @classmethod
    @my_decorator
    def class_method(cls):
        pass

    @property
    def value(self):
        return self._value
