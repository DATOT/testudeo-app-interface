"use client";
import {
  Sidebar,
  SidebarCTA,
  SidebarItem,
  SidebarItemGroup,
  SidebarItems,
  SidebarLogo,
} from "flowbite-react";
import {
  PiChartPieFill,
  PiPlugsFill,
  PiPlugFill,
  PiPlugsConnectedFill,
} from "react-icons/pi";

function NavSidebar() {
  const connected: boolean = false;
  const connectButtonStyles = connected
    ? "bg-green-200 hover:bg-green-100"
    : "bg-yellow-200 hover:bg-yellow-100";
  const connectCTAStyles = connected ? "bg-green-100" : "bg-yellow-100";
  return (
    <Sidebar aria-label="sidebar">
      <SidebarLogo href="#" img="/favicon.ico" imgAlt="Testudeo logo">
        Testudeo
      </SidebarLogo>
      <SidebarItems>
        <SidebarItemGroup>
          <SidebarItem href="#" icon={PiChartPieFill}>
            Dashboard
          </SidebarItem>
        </SidebarItemGroup>
        <SidebarItemGroup>
          <SidebarItem
            href="/connect"
            icon={PiPlugFill}
            className={connectButtonStyles}
          >
            Kết nối robot
          </SidebarItem>
          <SidebarCTA className={connectCTAStyles}>
            {connected ? (
              <>
                Kết nối thành công <PiPlugsConnectedFill />
              </>
            ) : (
              <>
                Chưa kết nối <PiPlugsFill />
              </>
            )}
          </SidebarCTA>
        </SidebarItemGroup>
      </SidebarItems>
    </Sidebar>
  );
}

export default NavSidebar;
