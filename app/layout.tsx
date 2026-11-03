import "./globals.css";
import NavSidebar from "@/app/components/NavSidebar";
export default function RootLayout({ children }: LayoutProps<"/">) {
  return (
    <html lang="en" className={`h-full antialiased`}>
      <body className="flex h-screen p-2">
        <NavSidebar />

        <main className="overflow-y-auto w-full pl-2 bg-white">{children}</main>
      </body>
    </html>
  );
}
