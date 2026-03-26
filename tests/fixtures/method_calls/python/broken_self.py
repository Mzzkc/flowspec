class Broken:
    def run(self):
        return self.nonexistent()


def main():
    Broken().run()
