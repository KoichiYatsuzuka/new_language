namespace ArrowBridge;

/// <summary>
/// A simple calculator class for testing Arrow C# interop.
/// </summary>
public class Calculator
{
    private double _accumulated;

    public Calculator()
    {
        _accumulated = 0.0;
    }

    public Calculator(double initial)
    {
        _accumulated = initial;
    }

    public static int Add(int a, int b) => a + b;
    public static int Subtract(int a, int b) => a - b;
    public static int Multiply(int a, int b) => a * b;
    public static double Divide(double a, double b)
    {
        if (b == 0.0) throw new DivideByZeroException("Cannot divide by zero");
        return a / b;
    }

    public static double Power(double base_, double exp) => Math.Pow(base_, exp);
    public static double Sqrt(double x) => Math.Sqrt(x);

    // Instance methods using accumulated state
    public void Accumulate(double value) { _accumulated += value; }
    public double GetAccumulated() => _accumulated;
    public void Reset() { _accumulated = 0.0; }

    public static double Pi => Math.PI;
    public static double E => Math.E;
}
