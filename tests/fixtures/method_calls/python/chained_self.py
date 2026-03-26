class Pipeline:
    def run(self):
        return self.step1()

    def step1(self):
        return self.step2()

    def step2(self):
        return "done"


def main():
    Pipeline().run()
