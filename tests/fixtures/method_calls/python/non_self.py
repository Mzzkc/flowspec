class Service:
    def handle(self, other):
        return other.process()


def main():
    Service().handle(None)
