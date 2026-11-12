using System;
using System.Windows.Forms;

namespace P2PHost;

internal static class Program
{
    [STAThread]
    private static void Main()
    {
        ApplicationConfiguration.Initialize();
        Application.Run(new HostForm());
    }
}
