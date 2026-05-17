# Access control: public / private / protected

class Counter:
    public:
    mut label: str

    private:
    mut count: int

    protected:
    mut step: int

    fn __init__(mut self, label: str) -> None:
        self.label = label
        self.count = 0
        self.step = 1

    fn increment(mut self) -> None:
        self.count = self.count + self.step

    fn get_count(self) -> int:
        return self.count

    fn set_step(mut self, s: int) -> None:
        self.step = s

    fn show(self) -> None:
        # inside the class: all members accessible
        print(self.label)
        print(self.count)
        print(self.step)


mut c = Counter("hits")
print(c.label)       # public: OK -> "hits"
c.increment()
c.increment()
print(c.get_count()) # via method: OK -> 2
c.set_step(5)
c.increment()
print(c.get_count()) # -> 7
c.show()             # inside class: all OK -> hits, 7, 5
