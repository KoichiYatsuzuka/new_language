using System.Windows.Forms;
using System.Drawing;

namespace FormBridge;

/// <summary>一行テキスト入力ダイアログ</summary>
internal sealed class InputBoxForm : Form
{
    private readonly Label  _prompt;
    private readonly TextBox _input;
    private readonly Button  _okBtn;
    private readonly Button  _cancelBtn;

    public bool   Confirmed { get; private set; }
    public string Input     { get; private set; } = "";

    public InputBoxForm(string title, string prompt, string defaultValue = "")
    {
        Text            = title;
        ClientSize      = new Size(380, 130);
        FormBorderStyle = FormBorderStyle.FixedDialog;
        MaximizeBox     = false;
        MinimizeBox     = false;
        StartPosition   = FormStartPosition.CenterScreen;
        BackColor       = Color.WhiteSmoke;
        Font            = new Font("Segoe UI", 9.5f);

        _prompt = new Label
        {
            Text     = prompt,
            Left     = 12, Top  = 14,
            Width    = 354, Height = 22,
            ForeColor = Color.FromArgb(50, 50, 50),
        };

        _input = new TextBox
        {
            Text        = defaultValue,
            Left        = 12, Top   = 40,
            Width       = 354, Height = 26,
            BorderStyle = BorderStyle.FixedSingle,
            Font        = new Font("Segoe UI", 10f),
        };

        _okBtn = new Button
        {
            Text      = "OK",
            Left      = 198, Top  = 86,
            Width     = 80, Height = 30,
            BackColor = Color.FromArgb(0, 120, 215),
            ForeColor = Color.White,
            FlatStyle = FlatStyle.Flat,
            Font      = new Font("Segoe UI", 9.5f, FontStyle.Bold),
        };
        _okBtn.FlatAppearance.BorderSize = 0;

        _cancelBtn = new Button
        {
            Text      = "Cancel",
            Left      = 286, Top  = 86,
            Width     = 80, Height = 30,
            BackColor = Color.FromArgb(200, 200, 200),
            ForeColor = Color.FromArgb(50, 50, 50),
            FlatStyle = FlatStyle.Flat,
            Font      = new Font("Segoe UI", 9.5f),
        };
        _cancelBtn.FlatAppearance.BorderSize = 0;

        _okBtn.Click     += (_, _) => { Confirmed = true; Input = _input.Text; Close(); };
        _cancelBtn.Click += (_, _) => Close();

        // Enterキーで OK
        _input.KeyDown += (_, e) =>
        {
            if (e.KeyCode == Keys.Enter)
            {
                Confirmed = true;
                Input = _input.Text;
                Close();
            }
        };

        AcceptButton = _okBtn;
        Controls.AddRange(new Control[] { _prompt, _input, _okBtn, _cancelBtn });
    }
}
