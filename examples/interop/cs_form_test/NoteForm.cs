using System.Windows.Forms;
using System.Drawing;

namespace FormBridge;

/// <summary>ノート作成フォーム (タイトル + 本文 + Submit/Cancel)</summary>
internal sealed class NoteForm : Form
{
    private readonly Label _titleLabel;
    private readonly TextBox _titleBox;
    private readonly Label _contentLabel;
    private readonly TextBox _contentBox;
    private readonly Button _submitBtn;
    private readonly Button _cancelBtn;

    public bool Submitted { get; private set; }
    public string NoteTitle   { get; private set; } = "";
    public string NoteContent { get; private set; } = "";

    public NoteForm(string title = "Create Note",
                    string titlePrompt   = "Title:",
                    string contentPrompt = "Content:")
    {
        Text            = title;
        ClientSize      = new Size(440, 310);
        FormBorderStyle = FormBorderStyle.FixedDialog;
        MaximizeBox     = false;
        MinimizeBox     = false;
        StartPosition   = FormStartPosition.CenterScreen;
        BackColor       = Color.WhiteSmoke;
        Font            = new Font("Segoe UI", 9.5f);

        _titleLabel = new Label
        {
            Text     = titlePrompt,
            Left     = 14, Top  = 14,
            Width    = 410, Height = 20,
            Font     = new Font("Segoe UI", 9.5f, FontStyle.Bold),
            ForeColor = Color.FromArgb(50, 50, 50),
        };

        _titleBox = new TextBox
        {
            Left  = 14, Top   = 36,
            Width = 410, Height = 26,
            BorderStyle = BorderStyle.FixedSingle,
            Font = new Font("Segoe UI", 10f),
        };

        _contentLabel = new Label
        {
            Text  = contentPrompt,
            Left  = 14, Top   = 74,
            Width = 410, Height = 20,
            Font  = new Font("Segoe UI", 9.5f, FontStyle.Bold),
            ForeColor = Color.FromArgb(50, 50, 50),
        };

        _contentBox = new TextBox
        {
            Left        = 14, Top    = 96,
            Width       = 410, Height = 155,
            Multiline   = true,
            ScrollBars  = ScrollBars.Vertical,
            BorderStyle = BorderStyle.FixedSingle,
            Font        = new Font("Segoe UI", 10f),
        };

        _submitBtn = new Button
        {
            Text      = "Submit",
            Left      = 246, Top  = 265,
            Width     = 85, Height = 32,
            BackColor = Color.FromArgb(0, 120, 215),
            ForeColor = Color.White,
            FlatStyle = FlatStyle.Flat,
            Font      = new Font("Segoe UI", 9.5f, FontStyle.Bold),
        };
        _submitBtn.FlatAppearance.BorderSize = 0;

        _cancelBtn = new Button
        {
            Text      = "Cancel",
            Left      = 339, Top  = 265,
            Width     = 85, Height = 32,
            BackColor = Color.FromArgb(200, 200, 200),
            ForeColor = Color.FromArgb(50, 50, 50),
            FlatStyle = FlatStyle.Flat,
            Font      = new Font("Segoe UI", 9.5f),
        };
        _cancelBtn.FlatAppearance.BorderSize = 0;

        _submitBtn.Click += (_, _) =>
        {
            NoteTitle   = _titleBox.Text;
            NoteContent = _contentBox.Text;
            Submitted   = true;
            Close();
        };
        _cancelBtn.Click += (_, _) => Close();

        Controls.AddRange(new Control[]
        {
            _titleLabel, _titleBox,
            _contentLabel, _contentBox,
            _submitBtn, _cancelBtn
        });
    }
}
