using System;
using System.Collections.Generic;
using System.Windows.Forms;
using System.Drawing;

namespace FormBridge;

/// <summary>シンプルな TODO リスト管理フォーム</summary>
internal sealed class TodoForm : Form
{
    private readonly ListBox _list;
    private readonly TextBox _input;
    private readonly Button  _addBtn;
    private readonly Button  _removeBtn;
    private readonly Button  _doneBtn;
    private readonly Label   _titleLabel;
    private readonly Label   _inputLabel;

    public List<string> Tasks { get; } = new();

    public TodoForm(string title = "TODO Manager", IEnumerable<string>? initialTasks = null)
    {
        Text            = title;
        ClientSize      = new Size(420, 360);
        FormBorderStyle = FormBorderStyle.FixedDialog;
        MaximizeBox     = false;
        MinimizeBox     = false;
        StartPosition   = FormStartPosition.CenterScreen;
        BackColor       = Color.WhiteSmoke;
        Font            = new Font("Segoe UI", 9.5f);

        _titleLabel = new Label
        {
            Text      = title,
            Left      = 12, Top = 10,
            Width     = 395, Height = 24,
            Font      = new Font("Segoe UI", 12f, FontStyle.Bold),
            ForeColor = Color.FromArgb(30, 30, 30),
        };

        _list = new ListBox
        {
            Left         = 12, Top   = 44,
            Width        = 395, Height = 200,
            BorderStyle  = BorderStyle.FixedSingle,
            Font         = new Font("Segoe UI", 10f),
            SelectionMode = SelectionMode.One,
        };

        if (initialTasks != null)
        {
            foreach (var t in initialTasks)
            {
                _list.Items.Add(t);
                Tasks.Add(t);
            }
        }

        _inputLabel = new Label
        {
            Text  = "New task:",
            Left  = 12, Top   = 256,
            Width = 395, Height = 20,
            ForeColor = Color.FromArgb(60, 60, 60),
            Font  = new Font("Segoe UI", 9.5f, FontStyle.Bold),
        };

        _input = new TextBox
        {
            Left        = 12, Top   = 278,
            Width       = 298, Height = 28,
            BorderStyle = BorderStyle.FixedSingle,
            Font        = new Font("Segoe UI", 10f),
        };

        _addBtn = new Button
        {
            Text      = "Add",
            Left      = 318, Top  = 276,
            Width     = 89, Height = 30,
            BackColor = Color.FromArgb(0, 150, 80),
            ForeColor = Color.White,
            FlatStyle = FlatStyle.Flat,
            Font      = new Font("Segoe UI", 9.5f, FontStyle.Bold),
        };
        _addBtn.FlatAppearance.BorderSize = 0;

        _removeBtn = new Button
        {
            Text      = "Remove Selected",
            Left      = 12, Top  = 318,
            Width     = 150, Height = 30,
            BackColor = Color.FromArgb(200, 50, 50),
            ForeColor = Color.White,
            FlatStyle = FlatStyle.Flat,
            Font      = new Font("Segoe UI", 9.5f),
        };
        _removeBtn.FlatAppearance.BorderSize = 0;

        _doneBtn = new Button
        {
            Text      = "Done",
            Left      = 318, Top  = 318,
            Width     = 89, Height = 30,
            BackColor = Color.FromArgb(0, 120, 215),
            ForeColor = Color.White,
            FlatStyle = FlatStyle.Flat,
            Font      = new Font("Segoe UI", 9.5f, FontStyle.Bold),
        };
        _doneBtn.FlatAppearance.BorderSize = 0;

        _addBtn.Click += (_, _) =>
        {
            var text = _input.Text.Trim();
            if (string.IsNullOrEmpty(text)) return;
            _list.Items.Add(text);
            Tasks.Add(text);
            _input.Clear();
            _input.Focus();
        };

        _input.KeyDown += (_, e) =>
        {
            if (e.KeyCode == Keys.Enter) _addBtn.PerformClick();
        };

        _removeBtn.Click += (_, _) =>
        {
            int idx = _list.SelectedIndex;
            if (idx < 0) return;
            Tasks.RemoveAt(idx);
            _list.Items.RemoveAt(idx);
            if (_list.Items.Count > 0)
                _list.SelectedIndex = Math.Min(idx, _list.Items.Count - 1);
        };

        _doneBtn.Click += (_, _) => Close();

        Controls.AddRange(new Control[]
        {
            _titleLabel,
            _list,
            _inputLabel, _input, _addBtn,
            _removeBtn, _doneBtn
        });
    }
}
