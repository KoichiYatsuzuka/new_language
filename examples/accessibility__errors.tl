
# Access control errors: accessing private/protected from invalid scope

class MyClass:
    public:
    mut x: int

    private:
    mut y: int

    protected:
    mut z: int

    fn __init__(mut self) -> None:
        self.x = 1
        self.y = 2
        self.z = 3


let obj = MyClass()
# This should raise AccessError: 'y' is private
print(obj.y)
